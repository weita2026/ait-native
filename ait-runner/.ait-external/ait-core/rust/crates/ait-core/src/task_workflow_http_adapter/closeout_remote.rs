use super::helpers::*;
use super::*;
use crate::change_json::ChangeJson;
use crate::json_support::JsonValue as Value;
use crate::patchset_json::PatchsetJson;
use crate::task_json::TaskJson;
use crate::workflow_closeout_remote;
use std::thread::sleep;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct HttpWorkflowCloseoutRemote {
    manager: TaskWorkflowHttpClientManager,
    bound_task_id: Option<String>,
    bound_change_id: Option<String>,
    bound_change_ref: Option<String>,
}

mod attestation_adapter;
mod ci_adapter;
mod closeout_client;
mod history_promotion_adapter;
mod land_adapter;
mod patchset_adapter;
mod policy_adapter;
mod review_adapter;
