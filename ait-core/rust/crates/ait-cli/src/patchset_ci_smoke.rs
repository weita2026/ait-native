use crate::json_support::{
    encode_value_or, encode_value_to_vec, parse_value_error_string, parse_value_option,
    parse_value_or,
};
use crate::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonValue};
use std::collections::{BTreeSet, HashSet};
use std::env;
use std::fs;
use std::net::TcpListener;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tiny_http::{Header, Response, Server};

const PLAN_SOURCE_TOKEN_FORBIDDEN: &[(&str, &[&str])] = &[(
    "src/ait/cli/app.py",
    &["line_sync", "root_main_sync", "remote_main_sync"],
)];

const PLAN_SOURCE_REGEX_FORBIDDEN: &[(&str, &[&str])] = &[(
    "src/ait/cli/app.py",
    &[
        "plan sync::{0,400}--default-line",
        "--default-line::{0,400}plan sync",
    ],
)];

const RETIRED_RELEASE_PYTHON_PATHS: &[&str] = &[
    "src/ait/cli/commands/release.py",
    "src/ait/release_ops.py",
    "src/ait/release_readiness.py",
    "src/ait/release_artifact_builder.py",
];

const IGNORED_DIRS: &[&str] = &[
    ".ait",
    ".ait-server",
    ".git",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "venv",
];

const FIXTURE_BASE_SNAPSHOT_ID: &str = "SNP-000000000001";
const FIXTURE_FINAL_LOCAL_SNAPSHOT_ID: &str = "SNP-000000000003";

#[derive(Clone, Copy, Debug)]
struct Tg1Case {
    index: u16,
    local_node_id: &'static str,
    corpus_node_id: &'static str,
    check_id: &'static str,
}

const TG1_CASES: &[Tg1Case] = &[
    Tg1Case { index: 1, local_node_id: "cli/test_plan.py::test_plan_create_and_revise_commands_are_not_public", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_plan.py::test_plan_create_and_revise_commands_are_not_public", check_id: "plan_public_surface" },
    Tg1Case { index: 2, local_node_id: "cli/test_plan.py::test_plan_sync_default_line_option_is_not_public", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_plan.py::test_plan_sync_default_line_option_is_not_public", check_id: "plan_public_surface" },
    Tg1Case { index: 3, local_node_id: "cli/test_plan.py::test_plan_public_surface_omits_legacy_line_alignment_contract", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_plan.py::test_plan_public_surface_omits_legacy_line_alignment_contract", check_id: "plan_source_guard" },
    Tg1Case { index: 4, local_node_id: "cli/test_line_worktree.py::test_repo_root_remote_plan_sync_bypasses_active_root_worktree_guard", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_repo_root_remote_plan_sync_bypasses_active_root_worktree_guard", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 5, local_node_id: "cli/test_line_worktree.py::test_plan_sync_remote_from_task_bound_worktree_delegates_and_preserves_repo_root_tracked_paths", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_plan_sync_remote_from_task_bound_worktree_delegates_and_preserves_repo_root_tracked_paths", check_id: "plan_sync_lineage_only" },
    Tg1Case { index: 6, local_node_id: "cli/test_plan.py::test_task_create_can_pin_plan_lineage_and_land_change_in_strict_mode", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_plan.py::test_task_create_can_pin_plan_lineage_and_land_change_in_strict_mode", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 7, local_node_id: "cli/test_line_worktree.py::test_task_worktree_exposes_docs_via_symlink_but_keeps_markdown_out_of_execution_snapshot", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_task_worktree_exposes_docs_via_symlink_but_keeps_markdown_out_of_execution_snapshot", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 8, local_node_id: "cli/test_line_worktree.py::test_init_keeps_docs_sprints_readme_forbidden_as_bootstrap_surface", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_init_keeps_docs_sprints_readme_forbidden_as_bootstrap_surface", check_id: "init_sprint_readme_guard" },
    Tg1Case { index: 9, local_node_id: "cli/test_plan.py::test_plan_sync_docs_sprints_readme_is_forbidden", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_plan.py::test_plan_sync_docs_sprints_readme_is_forbidden", check_id: "sprint_readme_contract" },
    Tg1Case { index: 10, local_node_id: "cli/test_line_worktree.py::test_plan_sync_remote_from_task_worktree_docs_symlink_delegates_to_repo_root", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_plan_sync_remote_from_task_worktree_docs_symlink_delegates_to_repo_root", check_id: "plan_sync_lineage_only" },
    Tg1Case { index: 11, local_node_id: "test_local_content_decoupling.py::test_docs_sprints_readme_stays_lineage_only", corpus_node_id: "corpora/ait/full_repo/tests/test_local_content_decoupling.py::test_docs_sprints_readme_stays_lineage_only", check_id: "plan_sync_lineage_only" },
    Tg1Case { index: 12, local_node_id: "cli/test_line_worktree.py::test_plan_sync_non_docs_markdown_from_worktree_remains_rejected", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_plan_sync_non_docs_markdown_from_worktree_remains_rejected", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 13, local_node_id: "cli/test_line_worktree.py::test_plan_sync_local_from_worktree_docs_symlink_delegates_to_repo_root", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_plan_sync_local_from_worktree_docs_symlink_delegates_to_repo_root", check_id: "plan_sync_lineage_only" },
    Tg1Case { index: 14, local_node_id: "cli/test_line_worktree.py::test_plan_sync_remote_from_worktree_docs_symlink_stays_lineage_only", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_plan_sync_remote_from_worktree_docs_symlink_stays_lineage_only", check_id: "plan_sync_lineage_only" },
    Tg1Case { index: 15, local_node_id: "cli/test_land_workflow.py::test_remote_land_excludes_non_docs_root_markdown", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_land_workflow.py::test_remote_land_excludes_non_docs_root_markdown", check_id: "stable_remote_land_flow" },
    Tg1Case { index: 16, local_node_id: "cli/test_line_worktree.py::test_task_start_refuses_broad_repo_root_auto_sync_when_multiple_markdown_paths_are_dirty", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_task_start_refuses_broad_repo_root_auto_sync_when_multiple_markdown_paths_are_dirty", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 17, local_node_id: "cli/test_line_worktree.py::test_task_start_auto_syncs_authored_markdown_before_creating_initial_change", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_task_start_auto_syncs_authored_markdown_before_creating_initial_change", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 18, local_node_id: "cli/test_line_worktree.py::test_change_create_rejects_authored_markdown_workspace_dispatch", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_change_create_rejects_authored_markdown_workspace_dispatch", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 19, local_node_id: "cli/test_land_workflow.py::test_workflow_land_reports_publish_patchset_next_action", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_land_workflow.py::test_workflow_land_reports_publish_patchset_next_action", check_id: "stable_remote_land_flow" },
    Tg1Case { index: 20, local_node_id: "cli/test_land_workflow.py::test_workflow_land_reports_land_submit_then_task_complete", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_land_workflow.py::test_workflow_land_reports_land_submit_then_task_complete", check_id: "stable_remote_land_flow" },
    Tg1Case { index: 21, local_node_id: "cli/test_line_worktree.py::test_repo_root_change_create_guides_operator_to_matching_bound_task_worktree", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_repo_root_change_create_guides_operator_to_matching_bound_task_worktree", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 22, local_node_id: "cli/test_line_worktree.py::test_worktree_remove_clears_active_root_worktree_binding", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_worktree_remove_clears_active_root_worktree_binding", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 23, local_node_id: "cli/test_line_worktree.py::test_task_start_keeps_repo_root_guard_but_allows_dirty_task_bootstrap", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_task_start_keeps_repo_root_guard_but_allows_dirty_task_bootstrap", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 24, local_node_id: "cli/test_line_worktree.py::test_repo_root_patchset_publish_guides_operator_to_matching_bound_task_worktree", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_repo_root_patchset_publish_guides_operator_to_matching_bound_task_worktree", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 25, local_node_id: "cli/test_line_worktree.py::test_repo_root_land_submit_guides_operator_to_matching_bound_task_worktree", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_line_worktree.py::test_repo_root_land_submit_guides_operator_to_matching_bound_task_worktree", check_id: "root_worktree_plan_sync_guard" },
    Tg1Case { index: 26, local_node_id: "cli/test_workflow_land_batch.py::test_workflow_land_all_completed_local_promotes_completed_local_slice_through_native_remote_land", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_workflow_land_batch.py::test_workflow_land_all_completed_local_promotes_completed_local_slice_through_native_remote_land", check_id: "final_snapshot_remote_promotion_contract" },
    Tg1Case { index: 27, local_node_id: "cli/test_workflow_land_batch.py::test_workflow_land_all_completed_local_publishes_required_origin_plan_revision_only", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_workflow_land_batch.py::test_workflow_land_all_completed_local_publishes_required_origin_plan_revision_only", check_id: "final_snapshot_remote_promotion_contract" },
    Tg1Case { index: 28, local_node_id: "cli/test_workflow_land_batch.py::test_workflow_land_all_completed_local_auto_publishes_rebound_unpublished_local_plan_lineage", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_workflow_land_batch.py::test_workflow_land_all_completed_local_auto_publishes_rebound_unpublished_local_plan_lineage", check_id: "final_snapshot_remote_promotion_contract" },
    Tg1Case { index: 29, local_node_id: "cli/test_workflow_land_batch.py::test_workflow_land_all_completed_local_refreshes_stale_authoritative_patchset_from_repo_root", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_workflow_land_batch.py::test_workflow_land_all_completed_local_refreshes_stale_authoritative_patchset_from_repo_root", check_id: "final_snapshot_remote_promotion_contract" },
    Tg1Case { index: 30, local_node_id: "cli/test_workflow_land_batch.py::test_workflow_land_all_completed_local_converges_remote_main_across_stale_local_segments", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_workflow_land_batch.py::test_workflow_land_all_completed_local_converges_remote_main_across_stale_local_segments", check_id: "final_snapshot_remote_promotion_contract" },
    Tg1Case { index: 31, local_node_id: "cli/test_task_change.py::test_change_publish_refuses_when_local_base_advanced_after_local_main_land", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_task_change.py::test_change_publish_refuses_when_local_base_advanced_after_local_main_land", check_id: "stable_remote_land_flow" },
    Tg1Case { index: 32, local_node_id: "cli/test_workflow_land_batch.py::test_workflow_land_routes_local_change_ids_without_remote_sequence_collision", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_workflow_land_batch.py::test_workflow_land_routes_local_change_ids_without_remote_sequence_collision", check_id: "final_snapshot_remote_promotion_contract" },
    Tg1Case { index: 33, local_node_id: "cli/test_plan.py::test_plan_sync_remote_rekeys_equivalent_structured_local_duplicate_onto_remote_canonical_plan", corpus_node_id: "corpora/ait/full_repo/tests/cli/test_plan.py::test_plan_sync_remote_rekeys_equivalent_structured_local_duplicate_onto_remote_canonical_plan", check_id: "plan_sync_lineage_only" },
];

#[derive(Clone, Debug)]
pub struct LinkIssue {
    pub path: PathBuf,
    pub line_number: usize,
    pub target: String,
    pub resolved_path: PathBuf,
}

#[derive(Clone, Debug)]
struct CommandOutput {
    status: i32,
    stdout: String,
    stderr: String,
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    url: String,
    body: String,
}

#[derive(Default)]
struct FakeRemoteState {
    remote_head_snapshot_id: Option<String>,
    line_head_snapshot_ids: std::collections::BTreeMap<String, String>,
    archived_line_names: BTreeSet<String>,
    selected_patchset_id: Option<String>,
    selected_patchset_base_snapshot_id: Option<String>,
    selected_patchset_revision_snapshot_id: Option<String>,
    selected_change_id: Option<String>,
    published_change_task_ids: std::collections::BTreeMap<String, String>,
    ci_run_patchset_ids: BTreeSet<String>,
    ci_run_required_patchset_ids: BTreeSet<String>,
    attested_patchset_ids: BTreeSet<String>,
    last_submitted_change_id: Option<String>,
    last_submitted_patchset_id: Option<String>,
    closed_task_ids: BTreeSet<String>,
    history_promotion: Option<JsonValue>,
}

struct FakeRemote {
    base_url: String,
    log: Arc<Mutex<Vec<RecordedRequest>>>,
    state: Arc<Mutex<FakeRemoteState>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

mod cleanup;
mod fixture_bootstrap;
mod server_worker;
mod status_attestation_validation;
mod suite_execution;

use self::fixture_bootstrap::*;
use self::server_worker::*;
use self::status_attestation_validation::*;
pub use self::suite_execution::*;

#[cfg(test)]
mod tests;
