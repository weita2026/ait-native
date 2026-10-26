mod memory;
mod model;

pub use memory::{MemoryExternalResolverCall, MemoryExternalSnapshotResolver};
pub use model::{
    resolve_external_lockfile, ExternalResolutionOptions, ExternalSnapshotResolver,
    ExternalSnapshotSelection,
};
