#![allow(unused_imports)]

use crate::middle::read_model_contract::{
    object_text_field, optional_text_field, read_model_payload_object, ReadModelContract,
    ReadModelRowSetSpec, ReadModelRows,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet, HashMap};

mod contracts;
mod helpers;
mod inputs;
mod live_turn_pressure;
mod operator_metrics;
mod operator_readiness;
mod runtime_metrics;

pub use contracts::{
    operator_metrics_read_model_contract, runtime_metrics_read_model_contract,
    OPERATOR_METRICS_READ_MODEL_CONTRACT, OPERATOR_METRICS_ROW_SETS,
    RUNTIME_METRICS_READ_MODEL_CONTRACT, RUNTIME_METRICS_ROW_SETS,
};
pub use inputs::{OperatorMetricsInput, RuntimeMetricsInput};
pub use live_turn_pressure::{
    live_turn_pressure_summary_from_normalized, normalize_live_turn_metrics,
};
pub use operator_metrics::operator_metrics_read_model;
pub use operator_readiness::operator_readiness_read_model;
pub use runtime_metrics::runtime_metrics_read_model;
