use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::binary_runtime::BinaryServingServices;
use crate::operational_binary_runtime::{
    OperationalBinaryRuntime, NATIVE_JOB_REPOSITORY_CI_UNIX_PATH,
    NATIVE_JOB_REPOSITORY_CI_WINDOWS_PATH, NATIVE_JOB_V3_CONTRACT,
};
use crate::runtime_service::ServerRuntimeService;
use ait_server_core::foundation::agent_protocol::agent_server_protocol_version;
use ait_server_core::foundation::native_repositories::{
    require_remote_sync_line_update_authority, LineCloseRequest, LineUpdateRequest,
    NativeRepositoryError, NativeRepositoryErrorKind, NativeRepositoryService, RemoteSyncPlanJson,
    RepositoryCreateRequest, RetireRepositoryRequest, SnapshotExistsRequest, SnapshotExportQuery,
    ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE, ZSTD_BULK_TREE_PACK_MEDIA_TYPE,
};
use ait_server_core::foundation::remote_binary_db::{
    binary_db_runtime_error_kind, BinaryDbErrorKind,
};
use ait_server_core::foundation::server_binary_lifecycle::ServerBinaryLifecycleConfig;
use ait_server_core::foundation::server_operational_job_domain::WorkerJobKind;
use ait_server_core::foundation::server_workflow_store::ServerWorkflowStore;
use ait_server_core::foundation::workflow_artifacts::review_summary_from_rows;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{
        header::{CONTENT_TYPE, RETRY_AFTER},
        HeaderMap, HeaderValue, Method, Request, StatusCode,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use tokio::task;

mod common;
mod contracts;
mod errors;
mod native_plan_routes;
mod native_queue_routes;
mod native_repository_authority_routes;
mod native_workflow;
mod operational_routes;
mod state;

use common::*;
use contracts::*;
use errors::*;
use native_plan_routes::*;
use native_queue_routes::*;
use native_repository_authority_routes::*;
use operational_routes::*;
use state::*;

const NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES: usize = 2 * 1024 * 1024 * 1024;
const SERVICE_ENDPOINTS: &[&str] = &[
    "/healthz",
    "/v1/handshake",
    "/v1/native/capabilities",
    "/v1/native/repository-authorities",
    "/v1/native/repository-authorities/:repository_index",
    "/v1/native/repository-authorities/:repository_index/lines",
    "/v1/native/repository-authorities/:repository_index/tasks",
    "/v1/native/repository-authorities/:repository_index/changes",
    "/v1/native/repository-authorities/:repository_index/sprints",
    "/v1/native/repository-authorities/:repository_index/worker-jobs",
    "/v1/native/repository-authorities/:repository_index/retirement",
    "/v1/native/repository-restores",
    "/v1/native/repository-authorities/:repository_index/read/queue-summary",
    "/v1/native/repository-authorities/:repository_index/read/task-queue",
    "/v1/native/repository-authorities/:repository_index/read/reviewer-inbox",
    "/v1/native/worker-jobs:claim",
];

fn service_endpoints() -> Vec<String> {
    SERVICE_ENDPOINTS
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}

fn parse_suffixed_tail(tail: &str, suffix: &str, context: &str) -> Result<String, ApiError> {
    let normalized = tail.trim_matches('/');
    let value = normalized
        .strip_suffix(suffix)
        .ok_or_else(|| ApiError::not_found(format!("unknown {context} route")))?;
    if value.trim().is_empty() {
        return Err(ApiError::bad_request(format!(
            "{context} path segment is required."
        )));
    }
    Ok(value.to_string())
}

fn release_server_state(config: &ServerBinaryLifecycleConfig) -> ServerState {
    let operational_binary = Arc::new(
        OperationalBinaryRuntime::ensure_from_config(config).unwrap_or_else(|error| {
            panic!("ait-server Binary DB lifecycle validation failed: {error}")
        }),
    );
    let binary = BinaryServingServices::new(operational_binary.clone())
        .unwrap_or_else(|error| panic!("ait-server Binary DB serving services failed: {error}"));
    ServerState {
        service_endpoints: service_endpoints(),
        runtime_service: binary.runtime,
        workflow_service: binary.workflow,
        repository_service: binary.repository,
        operational_binary,
    }
}

pub fn build_router(config: ServerBinaryLifecycleConfig) -> Router {
    build_router_with_state(release_server_state(&config))
}

fn build_router_with_state(state: ServerState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/handshake", get(handshake))
        .route("/v1/native/capabilities", get(native_operational_capabilities))
        .route(
            "/v1/native/worker-jobs:claim",
            post(native_claim_next_worker_job),
        )
        .route(
            "/v1/native/repository-authorities",
            get(native_operational_repository_authorities)
                .post(native_register_operational_repository_authority),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index",
            get(native_operational_repository_authority).post(native_run_repository_ci),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/retirement",
            get(native_repository_retirement).post(native_begin_repository_retirement),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/retirement/files/*file_path",
            get(native_repository_retirement_file)
                .layer(DefaultBodyLimit::max(NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES)),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/retirement/purge",
            post(native_purge_retired_repository),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/retirement/abort",
            post(native_abort_repository_retirement),
        )
        .route(
            "/v1/native/repository-restores",
            post(native_begin_repository_restore),
        )
        .route(
            "/v1/native/repository-restores/:restore_token/files/*file_path",
            axum::routing::put(native_upload_repository_restore_file)
                .layer(DefaultBodyLimit::max(NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES)),
        )
        .route(
            "/v1/native/repository-restores/:restore_token/commit",
            post(native_commit_repository_restore),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/worker-jobs",
            get(native_operational_worker_jobs),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/worker-jobs/:worker_job_action",
            get(native_operational_worker_job).post(native_operational_worker_job_action),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/lines",
            get(native_list_repository_lines),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/lines/*line_tail",
            get(native_get_repository_line)
                .put(native_put_repository_line)
                .post(native_post_repository_line),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/snapshots:exists",
            post(native_repository_authority_snapshot_existence),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/snapshots/*snapshot_tail",
            get(native_get_snapshot_tail),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/remote-sync/zstd-bulk/plan",
            post(native_repository_authority_zstd_bulk_plan)
                .layer(DefaultBodyLimit::max(NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES)),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/remote-sync/zstd-bulk/commit",
            post(native_repository_authority_zstd_bulk_commit)
                .layer(DefaultBodyLimit::max(NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES)),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/remote-sync/zstd-bulk/import-manifests/:snapshot_id",
            get(native_repository_authority_get_zstd_import_manifest),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/remote-sync/zstd-bulk/pull-manifests",
            post(native_repository_authority_get_zstd_pull_manifest)
                .layer(DefaultBodyLimit::max(NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES)),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/remote-sync/zstd-bulk/object-packs/:pack_id",
            get(native_repository_authority_get_zstd_bulk_object_pack)
                .put(native_repository_authority_put_zstd_bulk_object_pack)
                .layer(DefaultBodyLimit::max(NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES)),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/remote-sync/zstd-bulk/tree-packs/:pack_id",
            get(native_repository_authority_get_zstd_bulk_tree_pack)
                .put(native_repository_authority_put_zstd_bulk_tree_pack)
                .layer(DefaultBodyLimit::max(NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES)),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/tasks",
            get(native_workflow::native_list_tasks).post(native_workflow::native_create_task),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/task-start",
            post(native_workflow::native_start_plan_bound_task),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/read/tasks/:task_id/audit",
            get(native_workflow::native_read_task_audit),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/read/queue-summary",
            get(native_read_repository_queue_summary),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/read/task-queue",
            get(native_read_repository_task_queue),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/read/reviewer-inbox",
            get(native_read_repository_reviewer_inbox),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/tasks/:task_ref",
            get(native_workflow::native_get_repository_authority_task)
                .post(native_workflow::native_repository_authority_task_action),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/changes",
            get(native_workflow::native_list_repository_authority_changes)
                .post(native_workflow::native_create_change),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/changes/:change_id/patchsets",
            get(native_workflow::native_list_repository_authority_patchsets)
                .post(native_workflow::native_publish_repository_authority_patchset),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/changes/:change_id/reviews",
            get(native_workflow::native_list_repository_authority_reviews)
                .post(native_workflow::native_record_repository_authority_review),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/changes/:change_tail",
            get(native_workflow::native_get_repository_authority_change)
                .post(native_workflow::native_repository_authority_change_action),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/patchsets/:patchset_id/attestation",
            get(native_workflow::native_get_repository_authority_attestation)
                .put(native_workflow::native_put_repository_authority_attestation),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/patchsets/:patchset_id/policy",
            get(native_workflow::native_get_repository_authority_policy),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/patchsets/:patchset_id",
            get(native_workflow::native_get_repository_authority_patchset)
                .post(native_workflow::native_repository_authority_patchset_action),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/read/patchsets/:patchset_id/ci-status",
            get(native_read_repository_authority_patchset_ci_status),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/lands/:submission_id",
            get(native_workflow::native_get_repository_authority_land),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/task-land",
            post(native_workflow::native_repository_authority_task_land),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/history-promotion:prepare",
            post(native_workflow::native_prepare_repository_authority_history_promotion),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/sprints",
            get(native_list_repository_plans).post(native_create_repository_plan),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/sprints/:plan_id",
            get(native_get_repository_plan).patch(native_update_repository_plan_status),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/sprints/:plan_id/revisions",
            get(native_list_repository_plan_revisions).post(native_revise_repository_plan),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/sprints/:plan_id/revisions/:plan_revision_id",
            get(native_get_repository_plan_revision),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/sprints/:plan_id/revisions/:plan_revision_id/artifacts",
            get(native_get_repository_plan_revision)
                .put(native_put_repository_plan_revision_artifacts),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/sprint-plan-ids/by-contains",
            post(native_find_plan_ids_by_contains),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/sprint-task-linkage/resolve",
            post(native_resolve_task_plan_linkage),
        )
        .route(
            "/v1/native/repository-authorities/:repository_index/read/plans/candidate-inputs",
            get(native_read_plan_candidate_inputs),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            repository_lifecycle_admission,
        ))
        .with_state(state)
}

async fn repository_lifecycle_admission<B>(
    State(state): State<ServerState>,
    request: Request<B>,
    next: Next<B>,
) -> Response
where
    B: Send + 'static,
{
    if matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        return next.run(request).await;
    }
    let path = request.uri().path();
    let prefix = "/v1/native/repository-authorities/";
    let Some(tail) = path.strip_prefix(prefix) else {
        return next.run(request).await;
    };
    let (repository_segment, remainder) = match tail.split_once('/') {
        Some(parts) => parts,
        None => {
            let repository_segment = repository_segment_for_mutation(tail);
            if let Some(repository_segment) = repository_segment {
                match parse_canonical_index(repository_segment, "repository_index").and_then(
                    |index| {
                        state
                            .operational_binary
                            .require_active_repository(index)
                            .map_err(map_lifecycle_error)
                    },
                ) {
                    Ok(()) => return next.run(request).await,
                    Err(error) => return error.into_response(),
                }
            }
            return next.run(request).await;
        }
    };
    if remainder == "retirement"
        || remainder == "retirement/purge"
        || remainder == "retirement/abort"
        || is_worker_job_drain_action(remainder)
    {
        return next.run(request).await;
    }
    match parse_canonical_index(repository_segment, "repository_index").and_then(|index| {
        state
            .operational_binary
            .require_active_repository(index)
            .map_err(map_lifecycle_error)
    }) {
        Ok(()) => next.run(request).await,
        Err(error) => error.into_response(),
    }
}

fn repository_segment_for_mutation(value: &str) -> Option<&str> {
    value
        .strip_suffix(":runCi")
        .or_else(|| (!value.is_empty()).then_some(value))
}

fn is_worker_job_drain_action(remainder: &str) -> bool {
    let Some(action) = remainder.strip_prefix("worker-jobs/") else {
        return false;
    };
    ["claim", "heartbeat", "complete", "fail"]
        .iter()
        .any(|operation| action.ends_with(&format!(":{operation}")))
}

async fn native_repository_authority_snapshot_existence(
    State(state): State<ServerState>,
    Path((repository_index, action)): Path<(String, String)>,
    Json(request): Json<SnapshotExistsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if action != ":exists" {
        return Err(ApiError::not_found(format!(
            "unknown Repository Snapshot action {action:?}"
        )));
    }
    let service = state.repository_service.clone();
    let payload = native_repository_authority_routes::run_repository_call(move || {
        service.snapshot_existence(&repository_index, request)
    })
    .await?;
    Ok(ok_json(payload))
}

async fn native_run_repository_ci(
    State(state): State<ServerState>,
    Path(repository_action): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let repository_index =
        parse_suffixed_tail(&repository_action, ":runCi", "Repository CI action")?;
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || runtime.run_repo_ci(&repository_index, &payload))
        .await
        .map_err(|error| {
            ApiError::internal(format!(
                "Binary Repository CI enqueue worker failed: {error}"
            ))
        })?;
    map_json_result(result)
}

fn task_workflow_detail_read_model_json(payload: &JsonValue) -> Result<JsonValue, String> {
    if payload.is_object() {
        Ok(payload.clone())
    } else {
        Err("task workflow detail input must be an object".to_string())
    }
}

#[cfg(test)]
#[path = "http/binary_tests.rs"]
mod tests;
