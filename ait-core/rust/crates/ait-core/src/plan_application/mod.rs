//! Plan-scoped service payload assembly remains concrete to the `plan` domain.
//! It should not be lifted into shared foundation until a second domain needs
//! the same orchestration shape honestly.

use crate::json_support::{JsonMap, JsonNumber, JsonValue};
use crate::plan_dispatch::{
    plan_candidates_payload, plan_dispatch_summary, plan_items_payload, DispatchPlanInput,
    DispatchPlanItemInput, DispatchRevisionInput, DispatchSummaryItem, DispatchTaskInput,
    LinkedTaskSummary, LocalPlanPublishShadow, PlanCandidatesPayload, PlanDispatchSummary,
    PlanItemsPayload,
};
use crate::plan_workflow_json::PlanWorkflowJson;

mod application_orchestration;
mod request_models;
mod response_projection;
mod text_rendering;
mod validation_helpers;

pub use self::application_orchestration::*;
pub use self::request_models::*;
use self::response_projection::*;
use self::text_rendering::*;
use self::validation_helpers::*;

#[cfg(test)]
mod tests;
