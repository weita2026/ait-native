mod fs;
mod materialization_hash_cache;
mod model;

pub use crate::external::bindings::{inspect_external_binding_paths, ExternalBindingCheckFact};
pub use crate::external::link::parse_external_local_link_overrides;
pub use fs::{
    inspect_external_filesystem_status_report, inspect_external_materialization,
    inspect_external_status_report, inspect_operational_external_projection_roots,
    ExternalFilesystemStatusReport,
};
pub use model::{
    build_external_status_report, ExternalCurrentSourceArtifactRole,
    ExternalCurrentSourceArtifactState, ExternalCurrentSourceArtifactStatus,
    ExternalCurrentSourceCoreStatus, ExternalDuplicateEntry, ExternalDuplicateGroup,
    ExternalDuplicatePolicy, ExternalMaterializationObservation,
    ExternalObservedMaterializationState, ExternalStatusEntry, ExternalStatusInput,
    ExternalStatusReport, ExternalStatusState, ExternalStatusSummary,
};
