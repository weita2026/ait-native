mod facade;
mod facts;
mod normalization;

pub use facade::{
    normalize_plan_backend_identity_with_diagnostics_probe,
    normalize_plan_diagnostics_compatibility_with_diagnostics_probe,
    normalize_plan_diagnostics_doctor_with_diagnostics_probe,
    normalize_plan_diagnostics_readiness_with_diagnostics_probe,
    normalize_plan_diagnostics_request_with_diagnostics_probe,
    normalize_plan_wheel_status_with_diagnostics_probe, DiagnosticsFoundation, DiagnosticsJson,
};
pub use facts::{
    build_plan_backend_identity_facts_json, build_plan_diagnostics_compatibility_status_json,
    build_plan_diagnostics_doctor_facts_json, build_plan_diagnostics_readiness_status_json,
    build_plan_storage_readiness_facts_json, build_plan_wheel_status_facts_json,
};
pub use normalization::{
    normalize_plan_backend_identity_payload_json,
    normalize_plan_diagnostics_compatibility_payload_json,
    normalize_plan_diagnostics_doctor_payload_json,
    normalize_plan_diagnostics_readiness_payload_json,
    normalize_plan_diagnostics_request_payload_json, normalize_plan_wheel_status_payload_json,
};

#[cfg(test)]
use crate::json_support::JsonValue;

#[cfg(test)]
use facts::build_wheel_status_payload_with_file_io_store;

#[cfg(test)]
mod tests;
