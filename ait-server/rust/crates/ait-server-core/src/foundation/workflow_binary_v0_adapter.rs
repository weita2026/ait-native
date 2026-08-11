//! Exact layout-1 Binary DB v0 workflow adapter.
//!
//! This adapter deliberately does not consult the retired workflow codec or
//! its generic JSON payload files. Public workflow identities and JSON views
//! are derived from the fixed v0 records and their narrowly typed payloads.

#[path = "workflow_binary_v0_adapter/store.rs"]
mod store;

#[cfg(test)]
#[path = "workflow_binary_v0_adapter/tests.rs"]
mod tests;

pub use store::{
    validate_frozen_server_workflow_v0, validate_server_workflow_v0, BinaryDbServerWorkflowV0Store,
};
