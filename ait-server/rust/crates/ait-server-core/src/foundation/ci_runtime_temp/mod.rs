use crate::foundation::ci_workspace_cleanup::{
    prune_runtime_temp_namespace_json, RuntimeTempPruneRequest,
};
use std::sync::atomic::AtomicU64;

const CI_RAM_MIN_AVAILABLE_BYTES_ENV_NAMES: [&str; 2] = [
    "AIT_NATIVE_SERVER_CI_RAM_MIN_AVAILABLE_BYTES",
    "AIT_CI_RAM_MIN_AVAILABLE_BYTES",
];
const CI_RAM_RECLAIM_TARGET_BYTES_ENV_NAMES: [&str; 2] = [
    "AIT_NATIVE_SERVER_CI_RAM_RECLAIM_TARGET_BYTES",
    "AIT_CI_RAM_RECLAIM_TARGET_BYTES",
];
const CI_RUNTIME_PRESSURE_PRUNE_NAMESPACES: [&str; 4] = [
    "patchset-ci",
    "repo-ci",
    "land-main-seed",
    "snapshot-materialize",
];
const CARGO_PROFILE_LOCK_NAMES: [&str; 3] =
    [".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock"];
const CARGO_BUILD_DIR_LEASE_NAME: &str = ".ait-ci-build-lease";
const CARGO_WORKSPACE_PATH_HASH_TEMPLATE: &str = "{workspace-path-hash}";
const MAX_CARGO_CACHE_DISCOVERY_DEPTH: usize = 8;
const PERSISTENT_RUNTIME_ROOT_ENV_NAMES: [&str; 3] = [
    "AIT_RUNTIME_ROOT",
    "AIT_NATIVE_SERVER_CI_TMP_ROOT",
    "AIT_NATIVE_SERVER_DATA",
];

static RUNTIME_SEQUENCE: AtomicU64 = AtomicU64::new(1);

mod cargo_cache;
mod ram_root;
mod runtime_paths;

pub use cargo_cache::{
    acquire_cargo_build_dir_lease, prune_obsolete_cargo_incremental_generations, CargoBuildDirLease,
};
pub use ram_root::{ci_ram_runtime_root_with_source, validated_ci_ram_runtime_root_with_source};
pub use runtime_paths::{ci_runtime_paths_from_request, CiRuntimePaths};

use cargo_cache::{
    filesystem_available_bytes, filesystem_capacity_bytes,
    reclaim_cargo_incremental_cache_with_available,
};
use runtime_paths::{detect_memory_root, nonempty_env_path, path_string};

#[cfg(test)]
use cargo_cache::{prune_obsolete_cargo_incremental_generations_in, try_lock_cargo_profile};
#[cfg(test)]
use ram_root::{
    default_ci_ram_reclaim_target_bytes, reclaim_ci_ram_capacity_with_available,
    validate_ci_ram_root_device_boundary, validate_ci_ram_root_path_boundary,
};
#[cfg(test)]
use runtime_paths::{reinitialize_pruned_managed_runtime_paths, unix_millis};

#[cfg(test)]
mod tests;
