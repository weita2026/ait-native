#[path = "patchset_ci_host/helpers.rs"]
mod helpers;

#[path = "patchset_ci_host/contract.rs"]
mod contract;

#[path = "patchset_ci_host/suite_catalog.rs"]
mod suite_catalog;

#[path = "patchset_ci_host/completion.rs"]
mod completion;

#[path = "patchset_ci_host/active_state.rs"]
mod active_state;

#[path = "patchset_ci_host/job_summary.rs"]
mod job_summary;

#[path = "patchset_ci_host/status_summary.rs"]
mod status_summary;

pub use active_state::patchset_ci_active_state_json;
pub use completion::patchset_ci_completion_json;
pub use contract::patchset_ci_contract_available_json;
pub use status_summary::{
    patchset_ci_embedded_status_summary_json, patchset_ci_status_summary_json,
};
pub use suite_catalog::patchset_ci_suite_catalog_json;

#[cfg(test)]
#[path = "patchset_ci_host/tests.rs"]
mod tests;
