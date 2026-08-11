mod cleanup;
mod helpers;
mod materialization;
mod overlayfs;
mod prepare;
mod process;
mod seed_paths;
mod sparse_copy_up;

pub use self::cleanup::ci_test_shard_cleanup_json;
pub(crate) use self::cleanup::ci_test_shard_cleanup_json_impl;
pub use self::prepare::ci_test_shard_prepare_json;
pub(crate) use self::prepare::ci_test_shard_prepare_json_impl;

const RUNTIME_MANIFEST_FILE: &str = ".ait-test-shard-runtime.json";
const CLEANUP_STRATEGY_REMOVE_SHARD_DIR: &str = "single_final_dirty_cleanup";
const CLEANUP_STRATEGY_PRESERVE_REPO_DIR: &str = "preserve_repo_dir_restore_main_seed";
