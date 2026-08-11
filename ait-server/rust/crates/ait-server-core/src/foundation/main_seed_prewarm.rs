mod config;
mod copy_repo;
mod helpers;
mod lock;
mod manifest;
mod paths;
mod request;
mod reuse;
mod steps;

pub use self::request::{
    ci_main_seed_prewarm_for_plan_json, ci_main_seed_prewarm_json, request_has_main_seed_prewarm,
};
pub(crate) use self::request::{
    ci_main_seed_prewarm_for_plan_json_impl, ci_main_seed_prewarm_json_impl,
};

const PREWARM_MANIFEST_FILE: &str = ".ait/main-seed-prewarm.json";
const PREWARM_LOG_DIR: &str = ".ait/prewarm-logs";
const DEFAULT_LOCK_TIMEOUT_MS: u64 = 30_000;
const LOCK_POLL_MS: u64 = 100;
