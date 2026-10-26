use crate::plan_http_client::{
    PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientManager, PlanHttpClientResult,
    PlanHttpClientStats,
};

pub type TaskWorkflowHttpClientConfig = PlanHttpClientConfig;
pub type TaskWorkflowHttpClientError = PlanHttpClientError;
pub type TaskWorkflowHttpClientResult<T> = PlanHttpClientResult<T>;
pub type TaskWorkflowHttpClientStats = PlanHttpClientStats;
pub type TaskWorkflowHttpClientManager = PlanHttpClientManager;
