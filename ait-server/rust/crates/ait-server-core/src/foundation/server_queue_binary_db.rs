use crate::foundation::remote_binary_db::{
    binary_db_runtime_error, BinaryDbError, BinaryDbIndexAppender, ServerRemoteBinaryDb,
};
use crate::foundation::server_workflow_store::ServerWorkflowStore;
use crate::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store;
use crate::middle::queue_read_model::{queue_summary_read_model, QueueReadModelInput};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

mod rows;
mod service;

use self::rows::*;

pub use self::service::BinaryDbServerWorkflowReadModelService;

#[cfg(test)]
use self::service::QUEUE_PROJECTION_MUTATION_QUIET_PERIOD;

#[cfg(test)]
mod tests;
