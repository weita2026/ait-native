use crate::foundation::workflow_artifacts::review_summary_from_rows;
use crate::middle::read_model_contract::{
    json_value_to_text, object_text_field, read_model_payload_object, ReadModelContract,
    ReadModelRowSetSpec, ReadModelRows,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const CHANGE_STATUS_LANDED: &str = "landed";
const CHANGE_STATUS_ARCHIVED: &str = "archived";

mod contracts;
mod helpers;
mod inputs;
mod repository_detail;
mod repository_index;
mod review_packets;
mod task_detail;
mod worker_status;

pub use contracts::{
    repository_detail_read_model_contract, repository_index_read_model_contract,
    repository_worker_status_read_model_contract, task_workflow_detail_read_model_contract,
    REPOSITORY_DETAIL_READ_MODEL_CONTRACT, REPOSITORY_DETAIL_ROW_SETS,
    REPOSITORY_INDEX_READ_MODEL_CONTRACT, REPOSITORY_INDEX_ROW_SETS,
    REPOSITORY_WORKER_STATUS_READ_MODEL_CONTRACT, REPOSITORY_WORKER_STATUS_ROW_SETS,
    TASK_WORKFLOW_DETAIL_READ_MODEL_CONTRACT, TASK_WORKFLOW_DETAIL_ROW_SETS,
};
pub use inputs::{
    RepositoryDetailInput, RepositoryIndexInput, RepositoryWorkerStatusInput,
    TaskWorkflowDetailInput,
};
pub use repository_detail::repository_detail_read_model;
pub use repository_index::repository_index_read_model;
pub use task_detail::task_workflow_detail_read_model;
pub use worker_status::repository_worker_status_read_model;
