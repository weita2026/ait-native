#![recursion_limit = "256"]

#[cfg(test)]
#[path = "../../../test_support.rs"]
mod workspace_test_support;

#[macro_export]
macro_rules! perfetto_range {
    ($name:literal) => {
        ait_core::perfetto_range!($name)
    };
}

pub mod agent_harness;
pub mod agent_surface;
pub mod auth_surface;
pub mod blame_surface;
pub mod config_surface;
pub mod doctor_surface;
pub(crate) mod external_readiness_gate;
pub mod external_surface;
pub mod init_surface;
pub mod install_surface;
pub(crate) mod json_support;
pub mod patchset_ci_smoke;
pub mod primitives;
pub mod release_surface;
pub mod remote_head_recovery;
pub(crate) mod remote_repository;
pub mod remote_surface;
pub mod render;
pub mod repo_surface;
pub mod repository_retirement;
pub mod runtime;
pub mod tag_surface;
pub mod task_land_contract;
pub(crate) mod task_worktree_layout;
pub mod test_surface;
pub mod workspace_lock;

#[cfg(test)]
mod storage_boundary_tests;
