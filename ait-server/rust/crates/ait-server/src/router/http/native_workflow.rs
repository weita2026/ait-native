use super::{map_json_result, parse_suffixed_tail, ApiError, ServerState};
use crate::binary_runtime::RoutedBinaryWorkflowStore;
use crate::runtime_service::ServerRuntimeService;
use ait_server_core::foundation::server_workflow_store::ServerWorkflowStore;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio::task;

#[path = "native_workflow/changes.rs"]
mod changes;
#[path = "native_workflow/governance.rs"]
mod governance;
#[path = "native_workflow/history_promotion.rs"]
mod history_promotion;
#[path = "native_workflow/patchsets.rs"]
mod patchsets;
#[path = "native_workflow/read_models.rs"]
mod read_models;
#[path = "native_workflow/task_land.rs"]
mod task_land;
#[path = "native_workflow/tasks.rs"]
mod tasks;

use tasks::{run_workflow_call, run_workflow_mutation};

fn repository_authority_workflow_store(
    workflow: &dyn ServerWorkflowStore,
    repository_index: &str,
) -> Result<(String, Arc<dyn ServerWorkflowStore>), String> {
    let repository_index = repository_index.trim();
    if repository_index.is_empty() {
        return Err("repository_index must be a non-empty string".to_string());
    }
    workflow
        .as_any()
        .downcast_ref::<RoutedBinaryWorkflowStore>()
        .ok_or_else(|| {
            "repository authority workflow routing requires the registry-backed Binary workflow service"
                .to_string()
        })?
        .store_for_repo(repository_index)
}

pub(super) use changes::{
    native_create_change, native_get_repository_authority_change,
    native_list_repository_authority_changes, native_repository_authority_change_action,
};
pub(super) use governance::{
    native_get_repository_authority_attestation, native_get_repository_authority_land,
    native_get_repository_authority_patchset, native_get_repository_authority_policy,
    native_list_repository_authority_reviews, native_put_repository_authority_attestation,
    native_record_repository_authority_review, native_repository_authority_patchset_action,
};
pub(super) use history_promotion::native_prepare_repository_authority_history_promotion;
pub(super) use patchsets::{
    native_list_repository_authority_patchsets, native_publish_repository_authority_patchset,
};
pub(super) use task_land::native_repository_authority_task_land;
pub(super) use tasks::{
    native_create_task, native_get_repository_authority_task, native_list_tasks,
    native_read_task_audit, native_repository_authority_task_action, native_start_plan_bound_task,
};
