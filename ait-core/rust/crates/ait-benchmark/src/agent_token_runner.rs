use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::agent_token::{
    extract_and_validate_claude_transcript_with_workflow_options,
    extract_and_validate_codex_transcript_with_workflow_options,
    load_agent_token_raw_run_summaries_with_allowed_missing,
    load_agent_token_run_summaries_with_allowed_missing, protocol_requires_claude_model_evidence,
    AgentTokenServedModelReport, AgentTokenTranscriptWorkflowOptions,
};
use crate::agent_token_replacement::{
    first_valid_unaccepted_run_id,
    load_agent_token_campaign_statistical_view_allowing_host_shutdown_partial,
    statistical_replacement_authorization, AgentTokenStatisticalReplacementAuthorization,
};
use crate::{
    build_agent_token_report, build_agent_token_run_adjudication, build_agent_token_schedule,
    capture_host_shutdown_observation, classify_host_shutdown_interruption, digest_workspace,
    extract_agent_token_secondary_metrics, host_shutdown_replacement_run_id, import_codex_usage,
    load_agent_token_campaign, load_agent_token_campaign_for_evidence,
    load_agent_token_campaign_statistical_view, load_agent_token_raw_run_summaries,
    load_agent_token_run_summaries, materialize_game_fixture, recognized_infrastructure_failure,
    render_agent_token_report_markdown, replacement_run_id, sha256_digest, write_json_new,
    write_text_new, AgentTokenAccountingProfile, AgentTokenAitSprintMode, AgentTokenBrowserReport,
    AgentTokenCampaignManifest, AgentTokenCommandTranscript, AgentTokenEnvironment,
    AgentTokenGitWorktreeMode, AgentTokenHostShutdownPairRecoverySelection,
    AgentTokenInfrastructurePairRecoverySelection, AgentTokenInfrastructureRecoveryArtifact,
    AgentTokenMode, AgentTokenReport, AgentTokenRunSummary, AgentTokenSchedule,
    AgentTokenScheduleEntry, AgentTokenStatisticalReplacementSelection,
    AGENT_TOKEN_BROWSER_REPORT_CONTRACT, AGENT_TOKEN_ENVIRONMENT_CONTRACT,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_CONTRACT,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_POLICY_REVISION, AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_REASON,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_SELECTION_FILE,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_CONTRACT,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_POLICY_REVISION,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_REASON, AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_SELECTION_FILE,
    AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION, AGENT_TOKEN_PROTOCOL_REVISION,
    AGENT_TOKEN_PROTOCOL_V1_JSON, AGENT_TOKEN_RECOVERED_SPAWN_CAMPAIGN_ID,
    AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX, AGENT_TOKEN_RECOVERED_SPAWN_REASON,
    AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID, AGENT_TOKEN_REPLACEMENT_SELECTION_CONTRACT,
    AGENT_TOKEN_REPLACEMENT_SELECTION_FILE, AGENT_TOKEN_RUN_SUMMARY_CONTRACT,
    AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION,
};

pub const AGENT_TOKEN_CAMPAIGN_EXECUTION_CONTRACT: &str = "ait-agent-token-benchmark-execution/v1";
pub const AGENT_TOKEN_CAMPAIGN_RESUME_CONTRACT: &str = "ait-agent-token-campaign-resume/v2";
pub const AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_EXECUTION_CONTRACT: &str =
    "ait-agent-token-infrastructure-pair-recovery-execution/v1";
pub const AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_EXECUTION_CONTRACT: &str =
    "ait-agent-token-host-shutdown-pair-recovery-execution/v1";
pub const AGENT_TOKEN_REPLACEMENT_EXECUTION_CONTRACT: &str =
    "ait-agent-token-statistical-replacement-execution/v1";
pub const AGENT_TOKEN_RUN_MANIFEST_CONTRACT: &str = "ait-agent-token-run-manifest/v1";
pub const AGENT_TOKEN_RUN_INDEX_CONTRACT: &str = "ait-agent-token-run-index/v1";
pub const AGENT_TOKEN_WORKFLOW_VERIFICATION_CONTRACT: &str =
    "ait-agent-token-workflow-verification/v1";
pub const AGENT_TOKEN_EXECUTOR_PREFLIGHT_CONTRACT: &str = "ait-agent-token-executor-preflight/v1";
pub const AGENT_TOKEN_EXECUTOR_PREFLIGHT_ENVIRONMENT_CONTRACT: &str =
    "ait-agent-token-executor-preflight-environment/v1";
pub const AGENT_TOKEN_EXECUTOR_PREFLIGHT_USAGE_CONTRACT: &str =
    "ait-agent-token-executor-preflight-usage/v1";
pub const AGENT_TOKEN_GIT_WORKTREE_PERMISSION_PREFLIGHT_CONTRACT: &str =
    "ait-agent-token-git-worktree-permission-preflight/v1";
pub const AGENT_TOKEN_GIT_START_STATE_PROOF_CONTRACT: &str =
    "ait-agent-token-git-start-state-proof/v1";
pub const AGENT_TOKEN_MANAGED_WORKTREE_LIFECYCLE_CONTRACT: &str =
    "ait-agent-token-managed-worktree-lifecycle/v1";
pub const AGENT_TOKEN_MODEL_REQUEST_STATE_CONTRACT: &str = "ait-agent-token-model-request-state/v1";
pub const AGENT_TOKEN_CODEX_PERMISSION_PROFILE_CONTRACT: &str =
    "ait-agent-token-codex-permission-profile/v1";
pub const AGENT_TOKEN_VALID_CANDIDATE_OUTCOME_CONTINUATION_POLICY: &str =
    "retain_valid_unaccepted_outcome_continue_exact_suffix_without_retry";
pub const AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT: usize = 30;
const CODEX_PERMISSION_PROFILE_NAME: &str = "ait_benchmark_local_v1";
const CODEX_PERMISSION_PROFILE_PARENT: &str = ":workspace";
const CODEX_PERMISSION_PROFILE_LABEL: &str =
    "permission-profile:ait_benchmark_local_v1(:workspace,no-network)";
const CODEX_ENABLED_FEATURE_OVERRIDES: &[&str] = &[];
const CODEX_DISABLED_FEATURE_OVERRIDES: &[&str] = &[];
const AIT_SPRINT_CARD_PATH: &str = "docs/sprints/benchmark_task.md";
const CLAUDE_ALLOWED_TOOLS: &[&str] = crate::agent_token::CLAUDE_MEASURED_TOOL_SURFACE;
const CLAUDE_DISALLOWED_TOOLS: &[&str] =
    &["WebFetch", "WebSearch", "Task", "NotebookEdit", "TodoWrite"];
/// Inline settings for measured Claude lanes: the bash sandbox denies all
/// external network egress while permitting loopback binding, matching the
/// campaign network policy `disabled_except_loopback` that the Codex
/// permission profile enforces on its side. Filesystem writes are scoped to
/// the working directory plus exactly the declared additional workspace
/// roots and Git write exceptions, and the unsandboxed-command escape hatch
/// is disabled so a denied operation fails closed instead of prompting.
fn claude_sandbox_settings_json(
    add_dirs: &[PathBuf],
    git_write_exceptions: &[PathBuf],
) -> Result<String, String> {
    let mut allow_write = Vec::new();
    for path in add_dirs.iter().chain(git_write_exceptions.iter()) {
        allow_write.push(path.to_str().ok_or_else(|| {
            format!(
                "Claude sandbox write path is not valid UTF-8: {}",
                path.display()
            )
        })?);
    }
    serde_json::to_string(&serde_json::json!({
        "sandbox": {
            "enabled": true,
            "allowUnsandboxedCommands": false,
            "network": {"allowLocalBinding": true},
            "filesystem": {"allowWrite": allow_write},
        }
    }))
    .map_err(|error| format!("Failed to encode Claude sandbox settings: {error}"))
}
const EXECUTOR_PREFLIGHT_PROMPT: &str = r#"This is an executor health preflight, not a repository task.

Execute exactly 30 separate shell-tool calls, sequentially. In every call, run exactly `/bin/pwd`. Do not combine calls, use a loop, add another shell command, inspect repository content, invoke Git or AIT, browse, or create, edit, rename, or delete any file. Stop immediately if any call fails. After all 30 calls succeed, reply with exactly `executor-preflight-pass`.
"#;

fn project_document_loading_label(project_doc_max_bytes: usize) -> String {
    if project_doc_max_bytes == 0 {
        "disabled_symmetrically_project_doc_max_bytes_0".to_string()
    } else {
        format!(
            "enabled_symmetrically_pilot_diagnostic_project_doc_max_bytes_{project_doc_max_bytes}"
        )
    }
}

fn codex_enabled_feature_overrides() -> Vec<String> {
    CODEX_ENABLED_FEATURE_OVERRIDES
        .iter()
        .map(|feature| (*feature).to_string())
        .collect()
}

fn codex_disabled_feature_overrides() -> Vec<String> {
    CODEX_DISABLED_FEATURE_OVERRIDES
        .iter()
        .map(|feature| (*feature).to_string())
        .collect()
}

/// The recorded feature-override evidence is executor-specific: Codex pins
/// its CLI feature overrides (currently none), while Claude pins the exact
/// tool allow/deny lists passed to the CLI so the enforced tool policy is
/// part of the frozen campaign evidence.
fn executor_enabled_feature_overrides(manifest: &AgentTokenCampaignManifest) -> Vec<String> {
    match manifest.runtime.executor {
        crate::agent_token::AgentTokenExecutor::Codex => codex_enabled_feature_overrides(),
        crate::agent_token::AgentTokenExecutor::Claude => CLAUDE_ALLOWED_TOOLS
            .iter()
            .map(|tool| format!("allowed-tool:{tool}"))
            .collect(),
    }
}

fn executor_disabled_feature_overrides(manifest: &AgentTokenCampaignManifest) -> Vec<String> {
    match manifest.runtime.executor {
        crate::agent_token::AgentTokenExecutor::Codex => codex_disabled_feature_overrides(),
        crate::agent_token::AgentTokenExecutor::Claude => CLAUDE_DISALLOWED_TOOLS
            .iter()
            .map(|tool| format!("disallowed-tool:{tool}"))
            .chain(std::iter::once("prompt-suggestions:false".to_string()))
            .collect(),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentTokenCampaignExecution {
    pub contract: &'static str,
    pub campaign_id: String,
    pub output_dir: PathBuf,
    pub git_worktree_permission_preflight_passed: bool,
    pub preflight_passed: bool,
    pub scheduled_pair_count: usize,
    pub requested_pair_count: usize,
    pub completed_pair_count: usize,
    pub scheduled_run_count: usize,
    pub executed_run_count: usize,
    pub accepted_run_count: usize,
    pub invalid_run_count: usize,
    pub failed_run_count: usize,
    pub stopped_early: bool,
    pub stop_reason: Option<String>,
    pub claim_eligible: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentTokenCampaignResume {
    pub contract: &'static str,
    pub campaign_id: String,
    pub campaign_dir: PathBuf,
    pub source_protocol_revision: String,
    pub controller_protocol_revision: String,
    pub adjudicator_revision: String,
    pub continuation_policy: String,
    pub runner_program: PathBuf,
    pub runner_sha256: String,
    pub scheduled_pair_count: usize,
    pub previous_pair_count: usize,
    pub requested_additional_pair_count: usize,
    pub added_run_count: usize,
    pub total_run_count: usize,
    pub raw_run_count: usize,
    pub infrastructure_recovery_performed: bool,
    pub host_shutdown_recovery_performed: bool,
    pub recovered_pair_count: usize,
    pub statistically_excluded_run_count: usize,
    pub adjudicated_run_count: usize,
    pub stopped_early: bool,
    pub stop_reason: Option<String>,
    pub claim_eligible: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentTokenReplacementExecution {
    pub contract: &'static str,
    pub campaign_id: String,
    pub policy_revision: String,
    pub campaign_dir: PathBuf,
    pub replacement_root: PathBuf,
    pub source_run_id: String,
    pub replacement_run_id: String,
    pub replacement_runner: PathBuf,
    pub replacement_runner_sha256: String,
    pub preflight_passed: bool,
    pub valid_attempt: bool,
    pub accepted_equivalent: bool,
    pub selection_activated: bool,
    pub claim_eligible: bool,
    pub failure_reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentTokenCodexPermissionProfile {
    pub contract: String,
    pub name: String,
    pub extends: String,
    pub network_enabled: bool,
    pub primary_workspace: String,
    pub additional_workspace_roots: Vec<String>,
    pub git_write_exceptions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenGitWorktreePermissionPreflightReport {
    pub contract: String,
    pub campaign_id: String,
    pub codex_version: String,
    pub git_version: String,
    pub permission_profile: AgentTokenCodexPermissionProfile,
    pub required_command_count: usize,
    pub executed_command_count: usize,
    pub successful_command_count: usize,
    pub main_clean: bool,
    pub registered_worktree_count: Option<usize>,
    pub temporary_branch_absent: bool,
    pub main_commit_count: Option<u64>,
    pub task_path_absent: bool,
    pub passed: bool,
    pub failure_reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentTokenExecutorPreflightUsage {
    pub contract: String,
    pub model_provider: String,
    pub model_id: String,
    pub model_revision: String,
    pub reasoning_effort: String,
    pub input_tokens: u64,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_input_tokens: Option<u64>,
    pub output_tokens: u64,
    pub reasoning_tokens: Option<u64>,
    pub provider_total_tokens: u64,
    pub completed_turns: usize,
    pub usage_provenance: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentTokenExecutorPreflightEnvironment {
    pub contract: String,
    pub captured_at: String,
    pub os: String,
    pub architecture: String,
    pub codex_version: String,
    pub model: crate::AgentTokenModelPin,
    pub sandbox: String,
    pub codex_permission_profile: String,
    pub codex_permission_profile_parent: String,
    pub network_policy: String,
    pub project_doc_max_bytes: usize,
    pub benchmark_enabled_feature_overrides: Vec<String>,
    pub benchmark_disabled_feature_overrides: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenExecutorPreflightReport {
    pub contract: String,
    pub campaign_id: String,
    pub required_command_count: usize,
    pub started_command_count: usize,
    pub observed_command_count: usize,
    pub distinct_command_count: usize,
    pub successful_command_count: usize,
    pub failed_command_count: usize,
    pub unexpected_command_count: usize,
    pub sequential_violation_count: usize,
    pub unexpected_tool_item_count: usize,
    pub file_change_item_count: usize,
    pub codex_exit_code: Option<i32>,
    pub codex_timed_out: bool,
    pub elapsed_ms: u64,
    pub initial_workspace_digest: String,
    pub final_workspace_digest: Option<String>,
    pub infrastructure_failure: Option<String>,
    pub usage: Option<AgentTokenExecutorPreflightUsage>,
    pub passed: bool,
    pub failure_reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenRunManifest {
    pub contract: String,
    pub campaign_id: String,
    pub run_id: String,
    pub workload_id: String,
    pub mode: AgentTokenMode,
    pub accounting_profile: String,
    pub attempt: usize,
    pub block_index: usize,
    pub randomized_order: usize,
    pub fixture_revision: String,
    pub fixture_content_digest: String,
    pub shared_task_prompt_digest: String,
    pub measured_prompt_digest: String,
    pub workspace: String,
    #[serde(default)]
    pub execution_workspace: String,
    #[serde(default)]
    pub ait_edit_root_mode: crate::agent_token::AgentTokenAitEditRootMode,
    #[serde(default)]
    pub git_worktree_mode: AgentTokenGitWorktreeMode,
    pub network_policy: String,
    pub tool_policy: String,
    pub codex_permission_profile: String,
    pub codex_permission_profile_parent: String,
    #[serde(default)]
    pub benchmark_enabled_feature_overrides: Vec<String>,
    #[serde(default)]
    pub benchmark_disabled_feature_overrides: Vec<String>,
    pub project_document_loading: String,
    #[serde(default)]
    pub project_doc_max_bytes: usize,
    pub workflow_mode: String,
    pub sprint_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprint_card_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprint_item_ref: Option<String>,
    pub ait_server_allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_start_state_proof: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentTokenGitStartStateProof {
    pub contract: String,
    pub campaign_id: String,
    pub run_id: String,
    pub captured_at: String,
    pub current_branch: Option<String>,
    pub head_oid: Option<String>,
    pub main_oid: Option<String>,
    pub status_porcelain: Option<String>,
    pub clean: bool,
    pub head_matches_main: bool,
    pub passed: bool,
    pub failure_reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenWorkflowVerification {
    pub contract: String,
    pub mode: AgentTokenMode,
    pub closed: bool,
    pub workflow_mode: String,
    #[serde(default)]
    pub ait_edit_root_mode: crate::agent_token::AgentTokenAitEditRootMode,
    #[serde(default)]
    pub git_worktree_mode: AgentTokenGitWorktreeMode,
    pub sprint_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprint_card_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprint_item_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprint_item_closed: Option<bool>,
    pub default_remote_present: bool,
    pub remote_count: Option<u64>,
    pub ait_server_configured: bool,
    pub workspace_dirty: Option<bool>,
    pub current_line: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_start_head_oid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_pre_merge_head_oid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_candidate_parent_oid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_lineage_matches_start: Option<bool>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentTokenManagedWorktreeLifecycle {
    pub contract: String,
    pub campaign_id: String,
    pub run_id: String,
    pub boundary: String,
    pub desktop_private_ipc_invoked: bool,
    pub worktree_path: String,
    pub provision_started_at: String,
    pub ready_at: String,
    pub provisioning_elapsed_ms: u64,
    pub model_started_at: Option<String>,
    pub model_completed_at: Option<String>,
    pub model_elapsed_ms: Option<u64>,
    pub closeout_started_at: Option<String>,
    pub closed_at: Option<String>,
    pub closeout_elapsed_ms: Option<u64>,
    pub starting_head_oid: String,
    pub worktree_ready_head_oid: Option<String>,
    pub candidate_head_oid: Option<String>,
    pub final_main_head_oid: Option<String>,
    pub worktree_created_before_model: bool,
    pub host_closeout_after_terminal_model_event: bool,
    pub host_commit_created: bool,
    pub main_fast_forwarded: bool,
    pub worktree_removed: bool,
    pub closed: bool,
    pub failure_reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentTokenModelRequestState {
    pub contract: String,
    pub run_id: String,
    pub executor: String,
    pub thread_id: Option<String>,
    pub turn_started_count: usize,
    pub turn_completed_count: usize,
    pub turn_failed_count: usize,
    pub usage_event_count: usize,
    pub model_reroute_event_count: usize,
    pub safety_event_count: usize,
    pub provider_error_event_count: usize,
    pub terminal_state: String,
    pub internal_provider_request_count: Option<usize>,
    pub internal_provider_request_count_authority: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenRunIndex {
    pub contract: String,
    pub campaign_id: String,
    pub generated_at: String,
    pub scheduled_run_count: usize,
    pub executed_run_count: usize,
    pub runs: Vec<AgentTokenRunIndexEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenRunIndexEntry {
    pub run_id: String,
    pub workload_id: String,
    pub mode: AgentTokenMode,
    pub attempt: usize,
    pub valid_attempt: bool,
    pub accepted_equivalent: bool,
    pub provider_total_tokens: Option<u64>,
    pub run_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct ExternalCommandEvent {
    sequence: usize,
    phase: String,
    program: String,
    args: Vec<String>,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

struct TimedProcessResult {
    exit_code: Option<i32>,
    timed_out: bool,
    elapsed_ms: u64,
}

struct AitBootstrap {
    worktree_add_dir: PathBuf,
}

#[derive(Default)]
struct ExecutorPreflightObservation {
    started_command_count: usize,
    observed_command_count: usize,
    distinct_command_count: usize,
    successful_command_count: usize,
    failed_command_count: usize,
    unexpected_command_count: usize,
    sequential_violation_count: usize,
    unexpected_tool_item_count: usize,
    file_change_item_count: usize,
    errors: Vec<String>,
}

fn is_exact_executor_preflight_command(command: &str) -> bool {
    let command = command.trim();
    if command == "/bin/pwd" {
        return true;
    }
    ["/bin/zsh", "/bin/bash", "/bin/sh"].iter().any(|shell| {
        ["-c", "-lc"].iter().any(|flag| {
            let prefix = format!("{shell} {flag} ");
            command.strip_prefix(&prefix).is_some_and(|payload| {
                let payload = payload.trim();
                payload == "/bin/pwd" || payload == "'/bin/pwd'" || payload == "\"/bin/pwd\""
            })
        })
    })
}

fn inspect_executor_preflight_events(path: &Path) -> ExecutorPreflightObservation {
    let mut observation = ExecutorPreflightObservation::default();
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            observation.errors.push(format!(
                "failed to read executor preflight events {}: {error}",
                path.display()
            ));
            return observation;
        }
    };
    let mut command_ids = BTreeSet::new();
    let mut active_command_ids = BTreeSet::new();
    for (index, line) in source.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(event) => event,
            Err(error) => {
                observation.errors.push(format!(
                    "executor preflight event line {} is invalid JSON: {error}",
                    index + 1
                ));
                continue;
            }
        };
        let event_type = event.get("type").and_then(serde_json::Value::as_str);
        let item_type = event
            .pointer("/item/type")
            .and_then(serde_json::Value::as_str);
        let item_id = event
            .pointer("/item/id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        match (event_type, item_type) {
            (Some("item.started"), Some("command_execution")) => {
                observation.started_command_count += 1;
                if item_id.is_empty() {
                    observation
                        .errors
                        .push("executor preflight started command item has no id".to_string());
                } else {
                    if !active_command_ids.is_empty() {
                        observation.sequential_violation_count += 1;
                    }
                    if !active_command_ids.insert(item_id.to_string()) {
                        observation.errors.push(format!(
                            "executor preflight command item {item_id} started more than once"
                        ));
                    }
                }
            }
            (Some("item.completed"), Some("command_execution")) => {
                observation.observed_command_count += 1;
                if item_id.is_empty() {
                    observation
                        .errors
                        .push("executor preflight command item has no id".to_string());
                } else {
                    command_ids.insert(item_id.to_string());
                    if !active_command_ids.remove(item_id) {
                        observation.errors.push(format!(
                            "executor preflight command item {item_id} completed without one active start"
                        ));
                    }
                }
                if event
                    .pointer("/item/exit_code")
                    .and_then(serde_json::Value::as_i64)
                    == Some(0)
                {
                    observation.successful_command_count += 1;
                } else {
                    observation.failed_command_count += 1;
                }
                let expected = event
                    .pointer("/item/command")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(is_exact_executor_preflight_command);
                if !expected {
                    observation.unexpected_command_count += 1;
                }
            }
            (Some("item.completed"), Some("file_change")) => {
                observation.file_change_item_count += 1;
                observation.unexpected_tool_item_count += 1;
            }
            (Some("item.completed"), Some("agent_message" | "reasoning")) => {}
            (Some("item.completed"), Some(_)) => {
                observation.unexpected_tool_item_count += 1;
            }
            _ => {}
        }
    }
    if !active_command_ids.is_empty() {
        observation.errors.push(format!(
            "executor preflight ended with {} active command items",
            active_command_ids.len()
        ));
    }
    observation.distinct_command_count = command_ids.len();
    observation
}

/// Claude counterpart of the executor preflight inspector. A Bash tool_use
/// event is a command start; the matching tool_result is its completion
/// (success unless is_error); Edit/Write tool_use events are file-change
/// items; any other tool_use is an unexpected tool item.
fn inspect_claude_executor_preflight_events(path: &Path) -> ExecutorPreflightObservation {
    let mut observation = ExecutorPreflightObservation::default();
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            observation.errors.push(format!(
                "failed to read executor preflight events {}: {error}",
                path.display()
            ));
            return observation;
        }
    };
    let mut command_ids = BTreeSet::new();
    let mut active_command_ids = BTreeSet::new();
    let mut command_texts = std::collections::BTreeMap::new();
    for (index, line) in source.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(event) => event,
            Err(error) => {
                observation.errors.push(format!(
                    "executor preflight event line {} is invalid JSON: {error}",
                    index + 1
                ));
                continue;
            }
        };
        match event.get("type").and_then(serde_json::Value::as_str) {
            Some("assistant") => {
                let Some(content) = event
                    .pointer("/message/content")
                    .and_then(serde_json::Value::as_array)
                else {
                    continue;
                };
                for block in content {
                    if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let name = block
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let id = block
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    match name {
                        "Bash" => {
                            observation.started_command_count += 1;
                            if id.is_empty() {
                                observation.errors.push(
                                    "executor preflight started command item has no id".to_string(),
                                );
                            } else {
                                if !active_command_ids.is_empty() {
                                    observation.sequential_violation_count += 1;
                                }
                                if !active_command_ids.insert(id.to_string()) {
                                    observation.errors.push(format!(
                                        "executor preflight command item {id} started more than once"
                                    ));
                                }
                                if let Some(command) = block
                                    .pointer("/input/command")
                                    .and_then(serde_json::Value::as_str)
                                {
                                    command_texts.insert(id.to_string(), command.to_string());
                                }
                            }
                        }
                        "Edit" | "Write" => {
                            observation.file_change_item_count += 1;
                            observation.unexpected_tool_item_count += 1;
                        }
                        _ => {
                            observation.unexpected_tool_item_count += 1;
                        }
                    }
                }
            }
            Some("user") => {
                let Some(content) = event
                    .pointer("/message/content")
                    .and_then(serde_json::Value::as_array)
                else {
                    continue;
                };
                for block in content {
                    if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_result")
                    {
                        continue;
                    }
                    let id = block
                        .get("tool_use_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    if id.is_empty()
                        || !(active_command_ids.contains(id) || command_texts.contains_key(id))
                    {
                        continue;
                    }
                    observation.observed_command_count += 1;
                    command_ids.insert(id.to_string());
                    if !active_command_ids.remove(id) {
                        observation.errors.push(format!(
                            "executor preflight command item {id} completed without one active start"
                        ));
                    }
                    if block
                        .get("is_error")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        observation.failed_command_count += 1;
                    } else {
                        observation.successful_command_count += 1;
                    }
                    let expected = command_texts
                        .get(id)
                        .is_some_and(|command| is_exact_executor_preflight_command(command));
                    if !expected {
                        observation.unexpected_command_count += 1;
                    }
                }
            }
            _ => {}
        }
    }
    if !active_command_ids.is_empty() {
        observation.errors.push(format!(
            "executor preflight ended with {} active command items",
            active_command_ids.len()
        ));
    }
    observation.distinct_command_count = command_ids.len();
    observation
}

fn inspect_executor_preflight_events_for(
    manifest: &AgentTokenCampaignManifest,
    path: &Path,
) -> ExecutorPreflightObservation {
    match manifest.runtime.executor {
        crate::agent_token::AgentTokenExecutor::Codex => inspect_executor_preflight_events(path),
        crate::agent_token::AgentTokenExecutor::Claude => {
            inspect_claude_executor_preflight_events(path)
        }
    }
}

pub(crate) fn preflight_usage_from_normalized(
    usage: &crate::NormalizedAgentTokenUsage,
) -> AgentTokenExecutorPreflightUsage {
    AgentTokenExecutorPreflightUsage {
        contract: AGENT_TOKEN_EXECUTOR_PREFLIGHT_USAGE_CONTRACT.to_string(),
        model_provider: usage.model_provider.clone(),
        model_id: usage.model_id.clone(),
        model_revision: usage.model_revision.clone(),
        reasoning_effort: usage.reasoning_effort.clone(),
        input_tokens: usage.input_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        cache_write_input_tokens: usage.cache_write_input_tokens,
        output_tokens: usage.output_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        provider_total_tokens: usage.provider_total_tokens,
        completed_turns: usage.completed_turns,
        usage_provenance: usage.usage_provenance.clone(),
    }
}

fn executor_preflight_failure_reasons(
    observation: &ExecutorPreflightObservation,
    process: &TimedProcessResult,
    initial_workspace_digest: &str,
    final_workspace_digest: Option<&str>,
    infrastructure_failure: Option<&str>,
    usage: Option<&crate::NormalizedAgentTokenUsage>,
    mut failure_reasons: Vec<String>,
) -> Vec<String> {
    failure_reasons.extend(observation.errors.iter().cloned());
    if let Some(reason) = infrastructure_failure {
        failure_reasons.push(format!(
            "executor preflight infrastructure unavailable: {reason}"
        ));
    }
    if process.timed_out {
        failure_reasons.push("executor preflight timed out".to_string());
    }
    if process.exit_code != Some(0) {
        failure_reasons.push(format!(
            "executor preflight Codex exited with {:?}",
            process.exit_code
        ));
    }
    if observation.started_command_count != AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT {
        failure_reasons.push(format!(
            "executor preflight observed {} command starts; expected {}",
            observation.started_command_count, AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT
        ));
    }
    if observation.observed_command_count != AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT {
        failure_reasons.push(format!(
            "executor preflight observed {} command items; expected {}",
            observation.observed_command_count, AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT
        ));
    }
    if observation.distinct_command_count != AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT {
        failure_reasons.push(format!(
            "executor preflight observed {} distinct command ids; expected {}",
            observation.distinct_command_count, AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT
        ));
    }
    if observation.successful_command_count != AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT
        || observation.failed_command_count != 0
    {
        failure_reasons.push(format!(
            "executor preflight recorded {} successful and {} failed command items",
            observation.successful_command_count, observation.failed_command_count
        ));
    }
    if observation.unexpected_command_count != 0 {
        failure_reasons.push(format!(
            "executor preflight recorded {} command items outside the exact /bin/pwd probe",
            observation.unexpected_command_count
        ));
    }
    if observation.sequential_violation_count != 0 {
        failure_reasons.push(format!(
            "executor preflight recorded {} overlapping command starts",
            observation.sequential_violation_count
        ));
    }
    if observation.unexpected_tool_item_count != 0 {
        failure_reasons.push(format!(
            "executor preflight recorded {} unexpected non-command tool items",
            observation.unexpected_tool_item_count
        ));
    }
    if observation.file_change_item_count != 0 {
        failure_reasons.push(format!(
            "executor preflight recorded {} file-change items",
            observation.file_change_item_count
        ));
    }
    match usage {
        Some(usage) if usage.completed_turns != 1 => failure_reasons.push(
            "executor preflight must contain exactly one completed provider turn".to_string(),
        ),
        None => failure_reasons.push("executor preflight provider usage is missing".to_string()),
        _ => {}
    }
    if final_workspace_digest != Some(initial_workspace_digest) {
        failure_reasons.push("executor preflight workspace content changed".to_string());
    }
    failure_reasons
}

fn run_executor_preflight(
    manifest: &AgentTokenCampaignManifest,
    campaign_dir: &Path,
    versions: &CapturedVersions,
) -> Result<
    (
        AgentTokenExecutorPreflightReport,
        AgentTokenGitWorktreePermissionPreflightReport,
    ),
    String,
> {
    let git_permission_preflight =
        run_git_worktree_permission_preflight(manifest, campaign_dir, versions)?;
    let workspace = tempfile::Builder::new()
        .prefix("ait-agent-token-executor-preflight-")
        .tempdir()
        .map_err(|error| format!("Failed to create executor preflight workspace: {error}"))?;
    let initial_workspace_digest = digest_workspace(workspace.path(), &[])?;
    write_text_new(
        &campaign_dir.join("executor-preflight-prompt.txt"),
        EXECUTOR_PREFLIGHT_PROMPT,
    )?;
    let permission_profile = build_codex_permission_profile(workspace.path(), &[], &[])?;
    write_json_new(
        &campaign_dir.join("executor-preflight-permission-profile.json"),
        &permission_profile,
    )?;
    let raw_events = campaign_dir.join("private/executor-preflight-events.raw.jsonl");
    let stderr = campaign_dir.join("private/executor-preflight.stderr.txt");
    let process = run_measured_agent(
        manifest,
        workspace.path(),
        &[],
        &[],
        EXECUTOR_PREFLIGHT_PROMPT,
        &raw_events,
        &stderr,
    )?;
    let final_workspace_digest = digest_workspace(workspace.path(), &[]).ok();
    let observation = inspect_executor_preflight_events_for(manifest, &raw_events);
    let mut usage_failure_reasons = Vec::new();
    if !git_permission_preflight.passed {
        usage_failure_reasons.push(format!(
            "Git worktree permission preflight failed: {}",
            git_permission_preflight.failure_reasons.join("; ")
        ));
    }
    let imported_usage = match import_executor_usage(
        manifest,
        &raw_events,
        &format!("{}-executor-preflight", manifest.campaign_id),
        "executor-preflight",
        AgentTokenMode::GitLinearSingleSession,
    ) {
        Ok(imported) => {
            if imported.provider_refusal {
                usage_failure_reasons
                    .push("executor preflight provider returned a refusal".to_string());
            }
            if imported
                .provider_stop_reason
                .as_deref()
                .is_some_and(|reason| reason != "end_turn")
            {
                usage_failure_reasons.push(format!(
                    "executor preflight provider stop reason is {:?}, expected end_turn",
                    imported.provider_stop_reason
                ));
            }
            Some(imported)
        }
        Err(error) => {
            usage_failure_reasons.push(format!(
                "executor preflight provider usage is invalid: {error}"
            ));
            None
        }
    };
    let transcript = AgentTokenCommandTranscript {
        contract: crate::AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
        run_id: format!("{}-executor-preflight", manifest.campaign_id),
        mode: AgentTokenMode::GitLinearSingleSession,
        accounting_profile: manifest.accounting_profile,
        command_count: observation.observed_command_count,
        commands: Vec::new(),
        valid: true,
        errors: Vec::new(),
        observed_required_commands: Vec::new(),
    };
    let infrastructure_failure = classify_executor_infrastructure_failure(
        manifest,
        &raw_events,
        &stderr,
        &process,
        &transcript,
        imported_usage.as_ref().map(|imported| &imported.usage),
    );
    let failure_reasons = executor_preflight_failure_reasons(
        &observation,
        &process,
        &initial_workspace_digest,
        final_workspace_digest.as_deref(),
        infrastructure_failure.as_deref(),
        imported_usage.as_ref().map(|imported| &imported.usage),
        usage_failure_reasons,
    );

    let usage = imported_usage
        .as_ref()
        .map(|imported| preflight_usage_from_normalized(&imported.usage));
    if let Some(usage) = &usage {
        write_json_line_new(&campaign_dir.join("executor-preflight-usage.jsonl"), usage)?;
    } else {
        write_text_new(&campaign_dir.join("executor-preflight-usage.jsonl"), "")?;
    }
    let environment = AgentTokenExecutorPreflightEnvironment {
        contract: AGENT_TOKEN_EXECUTOR_PREFLIGHT_ENVIRONMENT_CONTRACT.to_string(),
        captured_at: Utc::now().to_rfc3339(),
        os: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        codex_version: versions.codex.clone(),
        model: manifest.model.clone(),
        sandbox: CODEX_PERMISSION_PROFILE_LABEL.to_string(),
        codex_permission_profile: permission_profile.name,
        codex_permission_profile_parent: permission_profile.extends,
        network_policy: manifest.network_policy.clone(),
        project_doc_max_bytes: manifest.runtime.project_doc_max_bytes,
        benchmark_enabled_feature_overrides: executor_enabled_feature_overrides(manifest),
        benchmark_disabled_feature_overrides: executor_disabled_feature_overrides(manifest),
    };
    write_json_new(
        &campaign_dir.join("executor-preflight-environment.json"),
        &environment,
    )?;
    let report = AgentTokenExecutorPreflightReport {
        contract: AGENT_TOKEN_EXECUTOR_PREFLIGHT_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        required_command_count: AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT,
        started_command_count: observation.started_command_count,
        observed_command_count: observation.observed_command_count,
        distinct_command_count: observation.distinct_command_count,
        successful_command_count: observation.successful_command_count,
        failed_command_count: observation.failed_command_count,
        unexpected_command_count: observation.unexpected_command_count,
        sequential_violation_count: observation.sequential_violation_count,
        unexpected_tool_item_count: observation.unexpected_tool_item_count,
        file_change_item_count: observation.file_change_item_count,
        codex_exit_code: process.exit_code,
        codex_timed_out: process.timed_out,
        elapsed_ms: process.elapsed_ms,
        initial_workspace_digest,
        final_workspace_digest,
        infrastructure_failure,
        usage,
        passed: failure_reasons.is_empty(),
        failure_reasons,
    };
    write_json_new(
        &campaign_dir.join("executor-preflight-report.json"),
        &report,
    )?;
    Ok((report, git_permission_preflight))
}

fn protocol_requires_git_start_state_proof(protocol_revision: &str) -> bool {
    protocol_revision == AGENT_TOKEN_PROTOCOL_REVISION
        || protocol_revision == AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION
        || protocol_revision == crate::AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION
        || protocol_revision == crate::AGENT_TOKEN_PROMPTED_INSPECTION_PREDECESSOR_PROTOCOL_REVISION
        || protocol_revision == crate::AGENT_TOKEN_PRE_REPLACEMENT_PROTOCOL_REVISION
        || protocol_revision == AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION
}

fn protocol_continues_valid_candidate_outcomes(protocol_revision: &str) -> bool {
    protocol_revision == AGENT_TOKEN_PROTOCOL_REVISION
        || protocol_revision == AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION
        || protocol_revision == crate::AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION
        || protocol_revision == crate::AGENT_TOKEN_PRE_REPLACEMENT_PROTOCOL_REVISION
        || protocol_revision == AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION
}

fn validate_resume_protocol_revision(protocol_revision: &str) -> Result<(), String> {
    if protocol_revision == AGENT_TOKEN_PROTOCOL_REVISION
        || protocol_revision == AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION
        || protocol_revision == crate::AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION
        || protocol_revision == crate::AGENT_TOKEN_PRE_REPLACEMENT_PROTOCOL_REVISION
        || protocol_revision == AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION
        || protocol_revision == crate::AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION
    {
        return Ok(());
    }
    Err(format!(
        "Campaign protocol {protocol_revision} is read-only and cannot resume; admitted revisions are {}, {}, {}, {}, {}, and {}",
        AGENT_TOKEN_PROTOCOL_REVISION,
        AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION,
        crate::AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION,
        crate::AGENT_TOKEN_PRE_REPLACEMENT_PROTOCOL_REVISION,
        AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION,
        crate::AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION
    ))
}

fn git_workflow_mode_label(mode: AgentTokenGitWorktreeMode) -> &'static str {
    match mode {
        AgentTokenGitWorktreeMode::AgentManaged => "git_local",
        AgentTokenGitWorktreeMode::CodexAppEquivalentManaged => "codex_app_equivalent_managed",
    }
}

fn run_manifest_execution_workspace_matches(
    protocol_revision: &str,
    recorded: &str,
    expected: &str,
) -> bool {
    recorded == expected
        || (protocol_revision == AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION
            && recorded.is_empty()
            && expected == "workspace")
}

fn git_write_exceptions_for_mode(
    mode: AgentTokenGitWorktreeMode,
    main_workspace: &Path,
    metadata: &Path,
    task_worktree: &Path,
) -> Vec<PathBuf> {
    let mut exceptions = vec![metadata.to_path_buf(), task_worktree.join(".git")];
    if mode == AgentTokenGitWorktreeMode::AgentManaged {
        exceptions.push(main_workspace.join(".git"));
    }
    exceptions
}

fn codex_run_permission_profile_matches(
    profile: &AgentTokenCodexPermissionProfile,
    run_mode: AgentTokenMode,
    git_worktree_mode: AgentTokenGitWorktreeMode,
) -> bool {
    if profile.contract != AGENT_TOKEN_CODEX_PERMISSION_PROFILE_CONTRACT
        || profile.name != CODEX_PERMISSION_PROFILE_NAME
        || profile.extends != CODEX_PERMISSION_PROFILE_PARENT
        || profile.network_enabled
    {
        return false;
    }
    if run_mode != AgentTokenMode::GitLinearSingleSession {
        return true;
    }
    let expected_git_write_exception_count = match git_worktree_mode {
        AgentTokenGitWorktreeMode::AgentManaged => 3,
        AgentTokenGitWorktreeMode::CodexAppEquivalentManaged => 2,
    };
    let primary_workspace = Path::new(&profile.primary_workspace);
    let additional_workspace_roots = profile
        .additional_workspace_roots
        .iter()
        .map(Path::new)
        .collect::<Vec<_>>();
    profile.additional_workspace_roots.len() == 2
        && profile.git_write_exceptions.len() == expected_git_write_exception_count
        && profile.git_write_exceptions.iter().all(|exception| {
            let exception = Path::new(exception);
            exception.starts_with(primary_workspace)
                || additional_workspace_roots
                    .iter()
                    .any(|root| exception.starts_with(root))
        })
}

fn validate_resume_prefix_outcomes(
    protocol_revision: &str,
    runs: &[AgentTokenRunSummary],
) -> Result<bool, String> {
    if runs.iter().any(|run| !run.valid_attempt) {
        return Err("Existing campaign prefix contains an unadjudicated invalid run".to_string());
    }
    let continue_valid_candidate_outcomes =
        protocol_continues_valid_candidate_outcomes(protocol_revision);
    if !continue_valid_candidate_outcomes && runs.iter().any(|run| !run.accepted_equivalent) {
        return Err(format!(
            "Existing campaign prefix contains a valid unaccepted run that protocol {protocol_revision} cannot continue"
        ));
    }
    Ok(continue_valid_candidate_outcomes)
}

fn current_runner_provenance() -> Result<(PathBuf, String), String> {
    let runner_program = std::env::current_exe()
        .map_err(|error| format!("Failed to resolve current benchmark runner: {error}"))?;
    let runner_program = fs::canonicalize(&runner_program).map_err(|error| {
        format!(
            "Failed to canonicalize benchmark runner {}: {error}",
            runner_program.display()
        )
    })?;
    let runner_bytes = fs::read(&runner_program).map_err(|error| {
        format!(
            "Failed to read benchmark runner {} for provenance: {error}",
            runner_program.display()
        )
    })?;
    Ok((runner_program, sha256_digest(&runner_bytes)))
}

fn execute_agent_token_pairs<F>(
    entries: &[AgentTokenScheduleEntry],
    requested_pair_count: usize,
    continue_valid_candidate_outcomes: bool,
    mut execute_lane: F,
) -> Result<(Vec<AgentTokenRunSummary>, Option<String>), String>
where
    F: FnMut(&AgentTokenScheduleEntry) -> Result<AgentTokenRunSummary, String>,
{
    let mut runs = Vec::new();
    let mut stop_reason = None;

    'pairs: for pair in entries.chunks_exact(2).take(requested_pair_count) {
        let pair_start = runs.len();
        for entry in pair {
            let run = execute_lane(entry)?;
            let infrastructure_failure = run.infrastructure_failure.clone();
            runs.push(run);
            if let Some(reason) = infrastructure_failure {
                stop_reason = Some(format!("{}: {reason}", entry.run_id));
                break 'pairs;
            }
        }
        if runs[pair_start..].iter().any(|run| !run.valid_attempt) {
            stop_reason = Some(format!(
                "{}/attempt {}: paired_invalid_attempt",
                pair[0].workload_id, pair[0].attempt
            ));
            break;
        }
        if !continue_valid_candidate_outcomes
            && runs[pair_start..]
                .iter()
                .any(|run| !run.accepted_equivalent)
        {
            stop_reason = Some(format!(
                "{}/attempt {}: legacy_paired_candidate_defect",
                pair[0].workload_id, pair[0].attempt
            ));
            break;
        }
    }

    Ok((runs, stop_reason))
}

pub fn run_agent_token_campaign(
    manifest_path: &Path,
    output_dir: &Path,
    max_pairs: Option<usize>,
) -> Result<AgentTokenCampaignExecution, String> {
    if max_pairs == Some(0) {
        return Err("max_pairs must be greater than zero when supplied".to_string());
    }
    let manifest = load_agent_token_campaign(manifest_path)?;
    if manifest.model.model_id.starts_with("REPLACE_")
        || manifest.model.model_revision.starts_with("REPLACE_")
    {
        return Err(
            "Campaign execution requires a real pinned model id and revision, not template placeholders"
                .to_string(),
        );
    }
    let schedule = build_agent_token_schedule(&manifest);
    if !schedule.entries.len().is_multiple_of(2) {
        return Err("Agent-token schedule does not contain complete two-lane pairs".to_string());
    }
    for pair in schedule.entries.chunks_exact(2) {
        let modes = pair.iter().map(|entry| entry.mode).collect::<BTreeSet<_>>();
        if pair[0].workload_id != pair[1].workload_id
            || pair[0].attempt != pair[1].attempt
            || modes
                != BTreeSet::from([
                    AgentTokenMode::GitLinearSingleSession,
                    AgentTokenMode::AitLinearSingleSession,
                ])
        {
            return Err("Agent-token schedule contains a non-atomic Git/AIT pair".to_string());
        }
    }
    let scheduled_pair_count = schedule.entries.len() / 2;
    let requested_pair_count = max_pairs.unwrap_or(scheduled_pair_count);
    if requested_pair_count > scheduled_pair_count {
        return Err(format!(
            "max_pairs {requested_pair_count} exceeds the {scheduled_pair_count} scheduled pairs"
        ));
    }
    let output_dir = prepare_campaign_output_directory(output_dir)?;
    fs::create_dir(output_dir.join("runs")).map_err(|error| {
        format!(
            "Failed to create campaign runs directory {}: {error}",
            output_dir.join("runs").display()
        )
    })?;

    copy_file_new(manifest_path, &output_dir.join("campaign-manifest.json"))?;
    copy_file_new(
        &manifest.runtime.fixture_manifest,
        &output_dir.join("fixture-manifest.json"),
    )?;
    let protocol = serde_json::from_str::<serde_json::Value>(AGENT_TOKEN_PROTOCOL_V1_JSON)
        .map_err(|error| format!("Compiled agent-token protocol is invalid: {error}"))?;
    write_json_new(&output_dir.join("protocol.json"), &protocol)?;
    write_json_new(&output_dir.join("randomization-schedule.json"), &schedule)?;

    let versions = capture_versions(&manifest)?;
    let (preflight, git_permission_preflight) =
        run_executor_preflight(&manifest, &output_dir, &versions)?;
    let mut runs = Vec::new();
    let mut stop_reason = (!preflight.passed).then(|| {
        format!(
            "executor_preflight_failed: {}",
            preflight.failure_reasons.join("; ")
        )
    });
    if preflight.passed {
        (runs, stop_reason) =
            execute_agent_token_pairs(&schedule.entries, requested_pair_count, true, |entry| {
                run_one(&manifest, entry, &output_dir, &versions)
            })?;
    }

    let index = AgentTokenRunIndex {
        contract: AGENT_TOKEN_RUN_INDEX_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        generated_at: Utc::now().to_rfc3339(),
        scheduled_run_count: schedule.entries.len(),
        executed_run_count: runs.len(),
        runs: runs
            .iter()
            .map(|run| AgentTokenRunIndexEntry {
                run_id: run.run_id.clone(),
                workload_id: run.workload_id.clone(),
                mode: run.mode,
                attempt: run.attempt,
                valid_attempt: run.valid_attempt,
                accepted_equivalent: run.accepted_equivalent,
                provider_total_tokens: run.usage.as_ref().map(|usage| usage.provider_total_tokens),
                run_summary: format!("runs/{}/run-summary.json", run.run_id),
                adjudication: None,
            })
            .collect(),
    };
    write_json_new(&output_dir.join("raw-run-index.json"), &index)?;
    let mut report = build_agent_token_report(&manifest, &schedule, &runs)?;
    let run_summary_paths = runs
        .iter()
        .map(|run| {
            (
                run.run_id.clone(),
                output_dir
                    .join("runs")
                    .join(&run.run_id)
                    .join("run-summary.json"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    enrich_agent_token_report_model_composition(&manifest, &runs, &run_summary_paths, &mut report)?;
    write_json_new(&output_dir.join("aggregate-report.json"), &report)?;
    write_json_new(
        &output_dir.join("comparison-report.json"),
        &serde_json::json!({
            "contract": "ait-agent-token-mode-comparison-report/v3",
            "campaign_id": &report.campaign_id,
            "protocol_revision": &report.protocol_revision,
            "campaign_scope": &report.campaign_scope,
            "git_worktree_permission_preflight_passed": git_permission_preflight.passed,
            "executor_preflight_passed": preflight.passed,
            "source_protocol_claim_eligible": report.source_protocol_claim_eligible,
            "current_policy_revision": &report.current_policy_revision,
            "current_policy_evaluation_mode": &report.current_policy_evaluation_mode,
            "current_policy_criteria_met": report.current_policy_criteria_met,
            "claim_eligible": report.claim_eligible,
            "served_models": &report.served_models,
            "mixed_model_run_count": report.mixed_model_run_count,
            "fallback_observed_run_count": report.fallback_observed_run_count,
            "pair_admission_policy": &report.pair_admission_policy,
            "comparisons": &report.comparisons,
            "blockers": &report.blockers,
            "current_policy_blockers": &report.current_policy_blockers,
        }),
    )?;
    let mut claim_boundary = render_agent_token_report_markdown(&report);
    claim_boundary.push_str(
        "\n## Claim Boundary\n\nSource-protocol claim eligibility is authoritative. A retrospective current-policy evaluation never changes the source campaign scope or makes an ineligible source campaign protocol-qualified. This campaign compares only the pinned game-development workloads, model, accounting profile, and single-session local topology. Its executor-preflight tokens are admission overhead and are excluded from AIT/Git metrics. It does not connect to `ait-server` and does not support a general AIT-versus-Git product claim.\n",
    );
    write_text_new(&output_dir.join("claim-boundary.md"), &claim_boundary)?;
    let stopped_early = stop_reason.is_some() || runs.len() != requested_pair_count * 2;

    Ok(AgentTokenCampaignExecution {
        contract: AGENT_TOKEN_CAMPAIGN_EXECUTION_CONTRACT,
        campaign_id: manifest.campaign_id,
        output_dir,
        git_worktree_permission_preflight_passed: git_permission_preflight.passed,
        preflight_passed: preflight.passed,
        scheduled_pair_count,
        requested_pair_count,
        completed_pair_count: runs.len() / 2,
        scheduled_run_count: schedule.entries.len(),
        executed_run_count: runs.len(),
        accepted_run_count: runs.iter().filter(|run| run.accepted_equivalent).count(),
        invalid_run_count: runs.iter().filter(|run| !run.valid_attempt).count(),
        failed_run_count: runs
            .iter()
            .filter(|run| run.valid_attempt && !run.accepted_equivalent)
            .count(),
        stopped_early,
        stop_reason,
        claim_eligible: report.claim_eligible,
    })
}

pub fn run_agent_token_statistical_replacement(
    campaign_dir: &Path,
    source_run_id: &str,
    activate_existing: bool,
) -> Result<AgentTokenReplacementExecution, String> {
    let campaign_dir = fs::canonicalize(campaign_dir).map_err(|error| {
        format!(
            "Failed to resolve replacement source campaign {}: {error}",
            campaign_dir.display()
        )
    })?;
    let manifest =
        load_agent_token_campaign_for_evidence(&campaign_dir.join("campaign-manifest.json"))?;
    let authorization = statistical_replacement_authorization(&manifest, source_run_id)?;
    let evidence_errors = validate_agent_token_campaign_evidence(&manifest, &campaign_dir)?;
    if !evidence_errors.is_empty() {
        return Err(format!(
            "Source campaign failed immutable evidence validation before replacement: {}",
            evidence_errors.join("; ")
        ));
    }
    let selection_path = campaign_dir.join(AGENT_TOKEN_REPLACEMENT_SELECTION_FILE);
    if selection_path.exists() {
        return Err(format!(
            "Statistical replacement selection already exists: {}",
            selection_path.display()
        ));
    }
    let replacement_root = campaign_dir.join("statistical-replacements/replacement-0001");
    if replacement_root.exists() {
        if !activate_existing {
            return Err(format!(
                "Statistical replacement evidence already exists; pass --activate-existing to revalidate it without rerun: {}",
                replacement_root.display()
            ));
        }
        return activate_existing_agent_token_statistical_replacement(
            &campaign_dir,
            &manifest,
            authorization,
            source_run_id,
            &replacement_root,
        );
    }
    if activate_existing {
        return Err(format!(
            "No existing statistical replacement evidence is available to activate: {}",
            replacement_root.display()
        ));
    }

    let schedule =
        crate::load_agent_token_schedule(&campaign_dir.join("randomization-schedule.json"))?;
    let source_entry = schedule
        .entries
        .iter()
        .find(|entry| entry.run_id == source_run_id)
        .cloned()
        .ok_or_else(|| format!("Replacement source run {source_run_id} is absent from schedule"))?;
    let source_runs = load_agent_token_run_summaries_with_allowed_missing(
        &campaign_dir,
        Some(crate::AGENT_TOKEN_HOST_SHUTDOWN_INTERRUPTED_RUN_ID),
    )?;
    let source_run = source_runs
        .iter()
        .find(|run| run.run_id == source_run_id)
        .ok_or_else(|| format!("Replacement source run {source_run_id} is absent"))?;
    if !source_run.valid_attempt || source_run.accepted_equivalent {
        return Err("Replacement source run must be valid and unaccepted".to_string());
    }
    if manifest.functional_replacement_policy
        == crate::AgentTokenFunctionalReplacementPolicy::FirstValidUnacceptedLaneOnce
        && first_valid_unaccepted_run_id(&schedule, &source_runs) != Some(source_run_id)
    {
        return Err(
            "Prospective replacement must target the first valid unaccepted lane in frozen schedule order"
                .to_string(),
        );
    }

    let versions = capture_versions(&manifest)?;
    let source_environment = decode_json_file::<AgentTokenEnvironment>(
        &campaign_dir
            .join("runs")
            .join(source_run_id)
            .join("environment.json"),
        "replacement source environment",
    )?;
    for (label, expected, observed) in [
        (
            "executor",
            source_environment.codex_version.as_str(),
            versions.codex.as_str(),
        ),
        (
            "AIT",
            source_environment.ait_version.as_str(),
            versions.ait.as_str(),
        ),
        (
            "Git",
            source_environment.git_version.as_str(),
            versions.git.as_str(),
        ),
        (
            "Node",
            source_environment.node_version.as_str(),
            versions.node.as_str(),
        ),
    ] {
        if expected != observed {
            return Err(format!(
                "Statistical replacement {label} version drifted: expected {expected:?}, got {observed:?}"
            ));
        }
    }
    if source_environment.browser_version != versions.browser {
        return Err(format!(
            "Statistical replacement browser version drifted: expected {:?}, got {:?}",
            source_environment.browser_version, versions.browser
        ));
    }

    fs::create_dir_all(
        replacement_root
            .parent()
            .expect("replacement root has a parent"),
    )
    .map_err(|error| {
        format!(
            "Failed to create statistical replacement parent {}: {error}",
            replacement_root.display()
        )
    })?;
    fs::create_dir(&replacement_root).map_err(|error| {
        format!(
            "Failed to create statistical replacement root {}: {error}",
            replacement_root.display()
        )
    })?;
    fs::create_dir(replacement_root.join("runs")).map_err(|error| {
        format!("Failed to create statistical replacement runs directory: {error}")
    })?;
    for file in [
        "campaign-manifest.json",
        "fixture-manifest.json",
        "protocol.json",
    ] {
        copy_file_new(&campaign_dir.join(file), &replacement_root.join(file))?;
    }
    let mut replacement_entry = source_entry;
    replacement_entry.run_id = authorization.replacement_run_id.to_string();
    write_json_new(
        &replacement_root.join("replacement-entry.json"),
        &replacement_entry,
    )?;
    let (runner_program, runner_sha256) = current_runner_provenance()?;
    copy_file_new(
        &runner_program,
        &replacement_root.join("replacement-runner"),
    )?;
    let (preflight, _) = run_executor_preflight(&manifest, &replacement_root, &versions)?;
    if !preflight.passed {
        let result = AgentTokenReplacementExecution {
            contract: AGENT_TOKEN_REPLACEMENT_EXECUTION_CONTRACT,
            campaign_id: manifest.campaign_id,
            policy_revision: authorization.policy_revision.to_string(),
            campaign_dir,
            replacement_root: replacement_root.clone(),
            source_run_id: source_run_id.to_string(),
            replacement_run_id: replacement_entry.run_id,
            replacement_runner: runner_program,
            replacement_runner_sha256: runner_sha256,
            preflight_passed: false,
            valid_attempt: false,
            accepted_equivalent: false,
            selection_activated: false,
            claim_eligible: false,
            failure_reasons: preflight.failure_reasons,
        };
        write_json_new(&replacement_root.join("result.json"), &result)?;
        return Ok(result);
    }

    let replacement_run = run_one(&manifest, &replacement_entry, &replacement_root, &versions)?;
    let source_summary_path = campaign_dir
        .join("runs")
        .join(source_run_id)
        .join("run-summary.json");
    let replacement_summary_path = replacement_root
        .join("runs")
        .join(&replacement_entry.run_id)
        .join("run-summary.json");
    let selection = AgentTokenStatisticalReplacementSelection {
        contract: AGENT_TOKEN_REPLACEMENT_SELECTION_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        policy_revision: authorization.policy_revision.to_string(),
        source_run_id: source_run_id.to_string(),
        source_run_summary_sha256: sha256_digest(&fs::read(&source_summary_path).map_err(
            |error| {
                format!(
                    "Failed to read replacement source summary {}: {error}",
                    source_summary_path.display()
                )
            },
        )?),
        replacement_run_id: replacement_entry.run_id.clone(),
        replacement_run_summary: format!(
            "statistical-replacements/replacement-0001/runs/{}/run-summary.json",
            replacement_entry.run_id
        ),
        replacement_run_summary_sha256: sha256_digest(
            &fs::read(&replacement_summary_path).map_err(|error| {
                format!(
                    "Failed to read replacement summary {}: {error}",
                    replacement_summary_path.display()
                )
            })?,
        ),
        replacement_runner_sha256: runner_sha256.clone(),
        reason: authorization.reason.clone(),
        selected_at: Utc::now().to_rfc3339(),
    };
    crate::agent_token_replacement::validate_selection_identity(&selection, &manifest)?;
    let admitted = crate::agent_token_replacement::validate_replacement_run(
        &manifest,
        &campaign_dir,
        source_run,
        &replacement_run,
        &selection,
    )
    .is_ok();
    if admitted {
        write_json_new(&selection_path, &selection)?;
    }

    let report = if admitted {
        refresh_campaign_derived_views(&manifest, &schedule, &source_runs, &campaign_dir)?
    } else {
        build_agent_token_report(&manifest, &schedule, &source_runs)?
    };
    let mut failure_reasons = replacement_run.failure_reasons.clone();
    if !admitted && failure_reasons.is_empty() {
        failure_reasons.push(
            "Replacement did not satisfy the exact statistical admission contract".to_string(),
        );
    }
    let result = AgentTokenReplacementExecution {
        contract: AGENT_TOKEN_REPLACEMENT_EXECUTION_CONTRACT,
        campaign_id: manifest.campaign_id,
        policy_revision: authorization.policy_revision.to_string(),
        campaign_dir,
        replacement_root: replacement_root.clone(),
        source_run_id: source_run_id.to_string(),
        replacement_run_id: replacement_entry.run_id,
        replacement_runner: runner_program,
        replacement_runner_sha256: runner_sha256,
        preflight_passed: true,
        valid_attempt: replacement_run.valid_attempt,
        accepted_equivalent: replacement_run.accepted_equivalent,
        selection_activated: admitted,
        claim_eligible: report.claim_eligible,
        failure_reasons,
    };
    write_json_new(&replacement_root.join("result.json"), &result)?;
    Ok(result)
}

fn activate_existing_agent_token_statistical_replacement(
    campaign_dir: &Path,
    manifest: &AgentTokenCampaignManifest,
    authorization: AgentTokenStatisticalReplacementAuthorization,
    source_run_id: &str,
    replacement_root: &Path,
) -> Result<AgentTokenReplacementExecution, String> {
    let activation_result_path = replacement_root.join("activation-result.json");
    if activation_result_path.exists() {
        return Err(format!(
            "Statistical replacement activation result already exists: {}",
            activation_result_path.display()
        ));
    }
    let prior_result = decode_json_file::<serde_json::Value>(
        &replacement_root.join("result.json"),
        "prior statistical replacement result",
    )?;
    if prior_result
        .get("contract")
        .and_then(serde_json::Value::as_str)
        != Some(AGENT_TOKEN_REPLACEMENT_EXECUTION_CONTRACT)
        || prior_result
            .get("campaign_id")
            .and_then(serde_json::Value::as_str)
            != Some(manifest.campaign_id.as_str())
        || prior_result
            .get("source_run_id")
            .and_then(serde_json::Value::as_str)
            != Some(source_run_id)
        || prior_result
            .get("replacement_run_id")
            .and_then(serde_json::Value::as_str)
            != Some(authorization.replacement_run_id.as_str())
        || prior_result
            .get("preflight_passed")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || prior_result
            .get("valid_attempt")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || prior_result
            .get("accepted_equivalent")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || prior_result
            .get("selection_activated")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
    {
        return Err(
            "Existing statistical replacement result is not the exact valid, accepted, non-activated evidence"
                .to_string(),
        );
    }

    let schedule =
        crate::load_agent_token_schedule(&campaign_dir.join("randomization-schedule.json"))?;
    let source_entry = schedule
        .entries
        .iter()
        .find(|entry| entry.run_id == source_run_id)
        .ok_or_else(|| format!("Replacement source run {source_run_id} is absent from schedule"))?;
    let replacement_entry = decode_json_file::<AgentTokenScheduleEntry>(
        &replacement_root.join("replacement-entry.json"),
        "existing statistical replacement entry",
    )?;
    if replacement_entry.run_id != authorization.replacement_run_id
        || replacement_entry.workload_id != source_entry.workload_id
        || replacement_entry.mode != source_entry.mode
        || replacement_entry.attempt != source_entry.attempt
        || replacement_entry.block_index != source_entry.block_index
        || replacement_entry.randomized_order != source_entry.randomized_order
    {
        return Err(
            "Existing statistical replacement entry differs from its exact source schedule lane"
                .to_string(),
        );
    }

    let source_runs = load_agent_token_run_summaries_with_allowed_missing(
        campaign_dir,
        Some(crate::AGENT_TOKEN_HOST_SHUTDOWN_INTERRUPTED_RUN_ID),
    )?;
    let source_run = source_runs
        .iter()
        .find(|run| run.run_id == source_run_id)
        .ok_or_else(|| format!("Replacement source run {source_run_id} is absent"))?;
    if !source_run.valid_attempt || source_run.accepted_equivalent {
        return Err("Replacement source run must be valid and unaccepted".to_string());
    }
    if manifest.functional_replacement_policy
        == crate::AgentTokenFunctionalReplacementPolicy::FirstValidUnacceptedLaneOnce
        && first_valid_unaccepted_run_id(&schedule, &source_runs) != Some(source_run_id)
    {
        return Err(
            "Prospective replacement activation must target the first valid unaccepted lane in frozen schedule order"
                .to_string(),
        );
    }
    let replacement_summary_path = replacement_root
        .join("runs")
        .join(&authorization.replacement_run_id)
        .join("run-summary.json");
    let replacement_run = decode_json_file::<AgentTokenRunSummary>(
        &replacement_summary_path,
        "existing statistical replacement run summary",
    )?;
    let source_summary_path = campaign_dir
        .join("runs")
        .join(source_run_id)
        .join("run-summary.json");
    let replacement_runner = replacement_root.join("replacement-runner");
    let replacement_runner_sha256 =
        sha256_digest(&fs::read(&replacement_runner).map_err(|error| {
            format!(
                "Failed to read existing replacement runner {}: {error}",
                replacement_runner.display()
            )
        })?);
    let selection = AgentTokenStatisticalReplacementSelection {
        contract: AGENT_TOKEN_REPLACEMENT_SELECTION_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        policy_revision: authorization.policy_revision.to_string(),
        source_run_id: source_run_id.to_string(),
        source_run_summary_sha256: sha256_digest(&fs::read(&source_summary_path).map_err(
            |error| {
                format!(
                    "Failed to read replacement source summary {}: {error}",
                    source_summary_path.display()
                )
            },
        )?),
        replacement_run_id: authorization.replacement_run_id.to_string(),
        replacement_run_summary: format!(
            "statistical-replacements/replacement-0001/runs/{}/run-summary.json",
            authorization.replacement_run_id
        ),
        replacement_run_summary_sha256: sha256_digest(
            &fs::read(&replacement_summary_path).map_err(|error| {
                format!(
                    "Failed to read existing replacement summary {}: {error}",
                    replacement_summary_path.display()
                )
            })?,
        ),
        replacement_runner_sha256: replacement_runner_sha256.clone(),
        reason: authorization.reason.clone(),
        selected_at: Utc::now().to_rfc3339(),
    };
    crate::agent_token_replacement::validate_selection_identity(&selection, manifest)?;
    crate::agent_token_replacement::validate_replacement_run(
        manifest,
        campaign_dir,
        source_run,
        &replacement_run,
        &selection,
    )?;

    let selection_path = campaign_dir.join(AGENT_TOKEN_REPLACEMENT_SELECTION_FILE);
    write_json_new(&selection_path, &selection)?;
    let report = refresh_campaign_derived_views(manifest, &schedule, &source_runs, campaign_dir)?;
    let prospective_policy = protocol_requires_claude_model_evidence(&manifest.protocol_revision)
        && manifest.functional_replacement_policy
            == crate::AgentTokenFunctionalReplacementPolicy::FirstValidUnacceptedLaneOnce;
    if !prospective_policy && !report.claim_eligible {
        return Err(
            "Existing statistical replacement passed artifact validation but did not produce a claim-eligible effective view"
                .to_string(),
        );
    }
    let result = AgentTokenReplacementExecution {
        contract: AGENT_TOKEN_REPLACEMENT_EXECUTION_CONTRACT,
        campaign_id: manifest.campaign_id.clone(),
        policy_revision: authorization.policy_revision.to_string(),
        campaign_dir: campaign_dir.to_path_buf(),
        replacement_root: replacement_root.to_path_buf(),
        source_run_id: source_run_id.to_string(),
        replacement_run_id: authorization.replacement_run_id.to_string(),
        replacement_runner,
        replacement_runner_sha256,
        preflight_passed: true,
        valid_attempt: true,
        accepted_equivalent: true,
        selection_activated: true,
        claim_eligible: report.claim_eligible,
        failure_reasons: Vec::new(),
    };
    write_json_new(&activation_result_path, &result)?;
    Ok(result)
}

pub fn resume_agent_token_campaign(
    campaign_dir: &Path,
    fixture_manifest: Option<&Path>,
    max_pairs: Option<usize>,
    adjudicate_transcripts: bool,
    adjudicate_recovered_spawn: bool,
    recover_infrastructure_pair: bool,
    recover_host_shutdown_pair: bool,
) -> Result<AgentTokenCampaignResume, String> {
    if max_pairs == Some(0) {
        return Err("max_pairs must be greater than zero when supplied".to_string());
    }
    let campaign_dir = fs::canonicalize(campaign_dir).map_err(|error| {
        format!(
            "Failed to resolve existing campaign directory {}: {error}",
            campaign_dir.display()
        )
    })?;
    let fixture_manifest = fixture_manifest
        .map(|path| require_resume_fixture_manifest_identity(&campaign_dir, path))
        .transpose()?;
    let manifest =
        crate::agent_token::load_agent_token_campaign_for_evidence_with_fixture_override(
            &campaign_dir.join("campaign-manifest.json"),
            fixture_manifest.as_deref(),
        )?;
    validate_resume_protocol_revision(&manifest.protocol_revision)?;
    let schedule =
        crate::load_agent_token_schedule(&campaign_dir.join("randomization-schedule.json"))?;
    validate_resume_schedule(&manifest, &schedule)?;
    if adjudicate_transcripts && adjudicate_recovered_spawn {
        return Err(
            "Transcript adjudication and recovered-spawn adjudication are distinct corrections"
                .to_string(),
        );
    }
    if adjudicate_recovered_spawn {
        append_recovered_spawn_adjudication(&campaign_dir, &manifest)?;
    }
    if recover_infrastructure_pair
        || recover_host_shutdown_pair
        || campaign_dir
            .join(AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_SELECTION_FILE)
            .is_file()
        || campaign_dir
            .join(AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_SELECTION_FILE)
            .is_file()
    {
        if adjudicate_transcripts {
            return Err(
                "Infrastructure pair recovery cannot be combined with transcript adjudication"
                    .to_string(),
            );
        }
        return resume_agent_token_campaign_with_infrastructure_recovery(
            campaign_dir,
            manifest,
            schedule,
            max_pairs,
            recover_infrastructure_pair,
            recover_host_shutdown_pair,
        );
    }

    let raw_runs = load_agent_token_raw_run_summaries(&campaign_dir)?;
    if adjudicate_transcripts {
        append_supported_run_adjudications(&campaign_dir, &manifest, &raw_runs)?;
    }
    let effective_runs = load_agent_token_run_summaries(&campaign_dir)?;
    let mut ordered_runs = exact_schedule_prefix(&schedule, effective_runs)?;
    let previous_run_count = ordered_runs.len();
    let previous_pair_count = previous_run_count / 2;
    let continue_valid_candidate_outcomes =
        validate_resume_prefix_outcomes(&manifest.protocol_revision, &ordered_runs)?;
    let prefix_errors = validate_agent_token_campaign_evidence_internal(
        &manifest,
        &campaign_dir,
        false,
        false,
        &BTreeSet::new(),
    )?;
    if !prefix_errors.is_empty() {
        return Err(format!(
            "Existing campaign prefix failed immutable evidence validation: {}",
            prefix_errors.join("; ")
        ));
    }

    let versions = capture_versions(&manifest)?;
    require_resume_version_identity(&campaign_dir, &ordered_runs, &versions)?;
    let scheduled_pair_count = schedule.entries.len() / 2;
    let remaining_pair_count = scheduled_pair_count.saturating_sub(previous_pair_count);
    let requested_additional_pair_count = max_pairs.unwrap_or(remaining_pair_count);
    if requested_additional_pair_count > remaining_pair_count {
        return Err(format!(
            "max_pairs {requested_additional_pair_count} exceeds the {remaining_pair_count} remaining pairs"
        ));
    }

    let (runner_program, runner_sha256) = current_runner_provenance()?;
    let resume_dir = next_resume_directory(&campaign_dir)?;
    write_json_new(
        &resume_dir.join("start.json"),
        &serde_json::json!({
            "contract": AGENT_TOKEN_CAMPAIGN_RESUME_CONTRACT,
            "campaign_id": manifest.campaign_id,
            "started_at": Utc::now().to_rfc3339(),
            "source_protocol_revision": manifest.protocol_revision,
            "controller_protocol_revision": AGENT_TOKEN_PROTOCOL_REVISION,
            "adjudicator_revision": crate::AGENT_TOKEN_ADJUDICATOR_REVISION,
            "continuation_policy": AGENT_TOKEN_VALID_CANDIDATE_OUTCOME_CONTINUATION_POLICY,
            "runner_program": runner_program,
            "runner_sha256": runner_sha256,
            "previous_run_count": previous_run_count,
            "previous_pair_count": previous_pair_count,
            "previous_valid_unaccepted_run_count": ordered_runs
                .iter()
                .filter(|run| run.valid_attempt && !run.accepted_equivalent)
                .count(),
            "requested_additional_pair_count": requested_additional_pair_count,
        }),
    )?;

    let start_entry = previous_run_count;
    let (added_runs, stop_reason) = execute_agent_token_pairs(
        &schedule.entries[start_entry..],
        requested_additional_pair_count,
        continue_valid_candidate_outcomes,
        |entry| run_one(&manifest, entry, &campaign_dir, &versions),
    )?;
    let added_run_count = added_runs.len();
    ordered_runs.extend(added_runs);
    let report =
        refresh_campaign_derived_views(&manifest, &schedule, &ordered_runs, &campaign_dir)?;
    let adjudicated_run_count = ordered_runs
        .iter()
        .filter(|run| {
            campaign_dir
                .join("adjudications")
                .join(format!("{}.json", run.run_id))
                .is_file()
        })
        .count();
    let stopped_early = stop_reason.is_some()
        || added_run_count != requested_additional_pair_count.saturating_mul(2);
    let result = AgentTokenCampaignResume {
        contract: AGENT_TOKEN_CAMPAIGN_RESUME_CONTRACT,
        campaign_id: manifest.campaign_id,
        campaign_dir,
        source_protocol_revision: manifest.protocol_revision,
        controller_protocol_revision: AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
        adjudicator_revision: crate::AGENT_TOKEN_ADJUDICATOR_REVISION.to_string(),
        continuation_policy: AGENT_TOKEN_VALID_CANDIDATE_OUTCOME_CONTINUATION_POLICY.to_string(),
        runner_program,
        runner_sha256,
        scheduled_pair_count,
        previous_pair_count,
        requested_additional_pair_count,
        added_run_count,
        total_run_count: ordered_runs.len(),
        raw_run_count: ordered_runs.len(),
        infrastructure_recovery_performed: false,
        host_shutdown_recovery_performed: false,
        recovered_pair_count: 0,
        statistically_excluded_run_count: 0,
        adjudicated_run_count,
        stopped_early,
        stop_reason,
        claim_eligible: report.claim_eligible,
    };
    write_json_new(&resume_dir.join("result.json"), &result)?;
    Ok(result)
}

fn require_resume_fixture_manifest_identity(
    campaign_dir: &Path,
    supplied_manifest: &Path,
) -> Result<PathBuf, String> {
    let frozen_path = campaign_dir.join("fixture-manifest.json");
    let frozen = fs::read(&frozen_path).map_err(|error| {
        format!(
            "Failed to read frozen campaign fixture manifest {}: {error}",
            frozen_path.display()
        )
    })?;
    let supplied_path = fs::canonicalize(supplied_manifest).map_err(|error| {
        format!(
            "Failed to resolve resume fixture manifest {}: {error}",
            supplied_manifest.display()
        )
    })?;
    let metadata = fs::symlink_metadata(&supplied_path).map_err(|error| {
        format!(
            "Failed to inspect resume fixture manifest {}: {error}",
            supplied_path.display()
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "Resume fixture manifest must resolve to a regular file: {}",
            supplied_path.display()
        ));
    }
    let supplied = fs::read(&supplied_path).map_err(|error| {
        format!(
            "Failed to read resume fixture manifest {}: {error}",
            supplied_path.display()
        )
    })?;
    if supplied != frozen {
        return Err(format!(
            "Resume fixture manifest differs from frozen campaign fixture bytes: frozen={}, supplied={}",
            sha256_digest(&frozen),
            sha256_digest(&supplied)
        ));
    }
    Ok(supplied_path)
}

fn exact_recovered_spawn_partial(
    manifest: &AgentTokenCampaignManifest,
    schedule: &AgentTokenSchedule,
    campaign_dir: &Path,
    effective_runs: &[AgentTokenRunSummary],
) -> bool {
    manifest.campaign_id == AGENT_TOKEN_RECOVERED_SPAWN_CAMPAIGN_ID
        && effective_runs.len() == AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX + 1
        && effective_runs.last().is_some_and(|run| {
            run.campaign_id == AGENT_TOKEN_RECOVERED_SPAWN_CAMPAIGN_ID
                && run.run_id == AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID
                && run.valid_attempt
                && run.infrastructure_failure.is_none()
                && run.usage.is_some()
                && run.transcript.valid
        })
        && schedule
            .entries
            .get(AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX)
            .is_some_and(|entry| entry.run_id == AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID)
        && schedule
            .entries
            .get(AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX + 1)
            .is_some_and(|entry| !campaign_dir.join("runs").join(&entry.run_id).exists())
        && campaign_dir
            .join("adjudications")
            .join(format!("{AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID}.json"))
            .is_file()
}

fn resume_agent_token_campaign_with_infrastructure_recovery(
    campaign_dir: PathBuf,
    manifest: AgentTokenCampaignManifest,
    schedule: AgentTokenSchedule,
    max_pairs: Option<usize>,
    recover_infrastructure_pair: bool,
    recover_host_shutdown_pair: bool,
) -> Result<AgentTokenCampaignResume, String> {
    let host_shutdown_selection_path =
        campaign_dir.join(AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_SELECTION_FILE);
    let allow_host_shutdown_partial =
        recover_host_shutdown_pair || host_shutdown_selection_path.is_file();
    // The contaminated pair is the trailing one the recovery is authorized to
    // replace. Its truncated lane would otherwise abort revalidation before the
    // recovery contract could act, so exempt exactly those two lanes. Every
    // other lane stays fully validated, and the set is empty unless this exact
    // authorization was supplied for a campaign with no prior selection.
    let selection_exists = campaign_dir
        .join(AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_SELECTION_FILE)
        .is_file();
    let exempt_run_ids = if recover_infrastructure_pair && !selection_exists {
        infrastructure_recovery_exempt_run_ids(&campaign_dir, &schedule)?
    } else {
        BTreeSet::new()
    };
    let prefix_errors = validate_agent_token_campaign_evidence_internal(
        &manifest,
        &campaign_dir,
        false,
        allow_host_shutdown_partial,
        &exempt_run_ids,
    )?;
    if !prefix_errors.is_empty() {
        return Err(format!(
            "Existing campaign prefix failed immutable evidence validation: {}",
            prefix_errors.join("; ")
        ));
    }
    let selection_path = campaign_dir.join(AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_SELECTION_FILE);
    let raw_runs = if allow_host_shutdown_partial {
        load_agent_token_raw_run_summaries_with_allowed_missing(
            &campaign_dir,
            Some(crate::AGENT_TOKEN_HOST_SHUTDOWN_INTERRUPTED_RUN_ID),
        )?
    } else {
        load_agent_token_raw_run_summaries(&campaign_dir)?
    };
    let versions = capture_versions(&manifest)?;
    let (runner_program, runner_sha256) = current_runner_provenance()?;
    let scheduled_pair_count = schedule.entries.len() / 2;

    let mut recovery_performed = false;
    let mut host_shutdown_recovery_performed = false;
    let mut recovered_spawn_partial = false;
    let previous_pair_count;
    let mut added_run_count = 0_usize;
    let start_entry;
    if host_shutdown_selection_path.is_file() {
        if recover_host_shutdown_pair {
            return Err("Host-shutdown recovery selection already exists".to_string());
        }
        let view = load_agent_token_campaign_statistical_view_allowing_host_shutdown_partial(
            &manifest,
            &schedule,
            &campaign_dir,
        )?;
        if view.infrastructure_recovery.is_none() || view.host_shutdown_recovery.is_none() {
            return Err(
                "Host-shutdown recovery selection did not produce the combined effective view"
                    .to_string(),
            );
        }
        if !view.effective_runs.len().is_multiple_of(2) {
            recovered_spawn_partial = exact_recovered_spawn_partial(
                &manifest,
                &schedule,
                &campaign_dir,
                &view.effective_runs,
            );
            if !recovered_spawn_partial {
                return Err("Host-shutdown recovered campaign ends inside a pair".to_string());
            }
        }
        require_resume_version_identity(&campaign_dir, &raw_runs, &versions)?;
        previous_pair_count = view.effective_runs.len() / 2;
        start_entry = view.effective_runs.len();
    } else if selection_path.is_file() {
        let view = if recover_host_shutdown_pair {
            load_agent_token_campaign_statistical_view_allowing_host_shutdown_partial(
                &manifest,
                &schedule,
                &campaign_dir,
            )?
        } else {
            load_agent_token_campaign_statistical_view(&manifest, &schedule, &campaign_dir)?
        };
        if view.infrastructure_recovery.is_none() {
            return Err(
                "Infrastructure recovery selection did not produce an effective recovery view"
                    .to_string(),
            );
        }
        if !view.effective_runs.len().is_multiple_of(2) {
            recovered_spawn_partial = exact_recovered_spawn_partial(
                &manifest,
                &schedule,
                &campaign_dir,
                &view.effective_runs,
            );
            if !recovered_spawn_partial {
                return Err("Recovered campaign effective prefix ends inside a pair".to_string());
            }
        }
        require_resume_version_identity(&campaign_dir, &raw_runs, &versions)?;
        previous_pair_count = view.effective_runs.len() / 2;
        if recover_host_shutdown_pair {
            added_run_count = execute_host_shutdown_pair_recovery(
                &manifest,
                &schedule,
                view.effective_runs.len(),
                &campaign_dir,
                &versions,
                &runner_program,
                &runner_sha256,
            )?;
            host_shutdown_recovery_performed = true;
            start_entry = view.effective_runs.len().saturating_add(2);
        } else {
            start_entry = view.effective_runs.len();
        }
    } else {
        if recover_host_shutdown_pair {
            return Err(
                "Host-shutdown recovery requires the prior infrastructure recovery selection"
                    .to_string(),
            );
        }
        if !recover_infrastructure_pair {
            return Err(
                "Campaign requires --recover-infrastructure-pair to authorize whole-pair recovery"
                    .to_string(),
            );
        }
        let (ordered_source_runs, pair_start) =
            classify_infrastructure_recovery_prefix(&schedule, raw_runs)?;
        require_resume_version_identity(&campaign_dir, &ordered_source_runs, &versions)?;
        previous_pair_count = pair_start / 2;
        start_entry = pair_start + 2;
        added_run_count = execute_infrastructure_pair_recovery(
            &manifest,
            &schedule,
            &ordered_source_runs,
            pair_start,
            &campaign_dir,
            &versions,
            &runner_program,
            &runner_sha256,
        )?;
        recovery_performed = true;
    }

    let remaining_pair_count = scheduled_pair_count.saturating_sub(previous_pair_count);
    let requested_additional_pair_count = max_pairs.unwrap_or(remaining_pair_count);
    if requested_additional_pair_count > remaining_pair_count {
        return Err(format!(
            "max_pairs {requested_additional_pair_count} exceeds the {remaining_pair_count} remaining pairs"
        ));
    }
    if (recovery_performed || host_shutdown_recovery_performed)
        && requested_additional_pair_count == 0
    {
        return Err("Whole-pair recovery requires at least one requested pair".to_string());
    }
    let suffix_pair_count =
        if recovery_performed || host_shutdown_recovery_performed || recovered_spawn_partial {
            requested_additional_pair_count.saturating_sub(1)
        } else {
            requested_additional_pair_count
        };

    let resume_dir = next_resume_directory(&campaign_dir)?;
    write_json_new(
        &resume_dir.join("start.json"),
        &serde_json::json!({
            "contract": AGENT_TOKEN_CAMPAIGN_RESUME_CONTRACT,
            "campaign_id": manifest.campaign_id,
            "started_at": Utc::now().to_rfc3339(),
            "source_protocol_revision": manifest.protocol_revision,
            "controller_protocol_revision": AGENT_TOKEN_PROTOCOL_REVISION,
            "adjudicator_revision": crate::AGENT_TOKEN_ADJUDICATOR_REVISION,
            "continuation_policy": if recovered_spawn_partial {
                AGENT_TOKEN_RECOVERED_SPAWN_REASON
            } else if host_shutdown_recovery_performed || host_shutdown_selection_path.is_file() {
                AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_REASON
            } else {
                AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_REASON
            },
            "runner_program": runner_program,
            "runner_sha256": runner_sha256,
            "previous_pair_count": previous_pair_count,
            "requested_additional_pair_count": requested_additional_pair_count,
            "infrastructure_recovery_performed": recovery_performed,
            "host_shutdown_recovery_performed": host_shutdown_recovery_performed,
            "recovered_spawn_partial_lane": recovered_spawn_partial,
            "partial_lane_start_entry": recovered_spawn_partial.then_some(start_entry),
            "suffix_start_entry": start_entry + usize::from(recovered_spawn_partial),
            "suffix_pair_count": suffix_pair_count,
        }),
    )?;

    let mut stop_reason = None;
    if recovered_spawn_partial {
        let entry = schedule.entries.get(start_entry).ok_or_else(|| {
            "Recovered-spawn continuation is missing its paired schedule entry".to_string()
        })?;
        let run = run_one(&manifest, entry, &campaign_dir, &versions)?;
        let infrastructure_failure = run.infrastructure_failure.clone();
        let valid_attempt = run.valid_attempt;
        added_run_count = added_run_count.saturating_add(1);
        if let Some(reason) = infrastructure_failure {
            stop_reason = Some(format!("{}: {reason}", entry.run_id));
        } else if !valid_attempt {
            stop_reason = Some(format!(
                "{}/attempt {}: paired_invalid_attempt",
                entry.workload_id, entry.attempt
            ));
        }
    }
    let (added_suffix_runs, suffix_stop_reason) = if stop_reason.is_none() {
        execute_agent_token_pairs(
            &schedule.entries[start_entry + usize::from(recovered_spawn_partial)..],
            suffix_pair_count,
            true,
            |entry| run_one(&manifest, entry, &campaign_dir, &versions),
        )?
    } else {
        (Vec::new(), None)
    };
    if stop_reason.is_none() {
        stop_reason = suffix_stop_reason;
    }
    added_run_count = added_run_count.saturating_add(added_suffix_runs.len());
    let raw_runs = if host_shutdown_selection_path.is_file() {
        load_agent_token_run_summaries_with_allowed_missing(
            &campaign_dir,
            Some(crate::AGENT_TOKEN_HOST_SHUTDOWN_INTERRUPTED_RUN_ID),
        )?
    } else {
        load_agent_token_run_summaries(&campaign_dir)?
    };
    let report = refresh_campaign_derived_views(&manifest, &schedule, &raw_runs, &campaign_dir)?;
    let adjudicated_run_count = raw_runs
        .iter()
        .filter(|run| {
            campaign_dir
                .join("adjudications")
                .join(format!("{}.json", run.run_id))
                .is_file()
        })
        .count();
    let expected_added_runs = requested_additional_pair_count
        .saturating_mul(2)
        .saturating_sub(usize::from(recovered_spawn_partial));
    let stopped_early = stop_reason.is_some() || added_run_count != expected_added_runs;
    let result = AgentTokenCampaignResume {
        contract: AGENT_TOKEN_CAMPAIGN_RESUME_CONTRACT,
        campaign_id: manifest.campaign_id,
        campaign_dir,
        source_protocol_revision: manifest.protocol_revision,
        controller_protocol_revision: AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
        adjudicator_revision: crate::AGENT_TOKEN_ADJUDICATOR_REVISION.to_string(),
        continuation_policy: if recovered_spawn_partial {
            AGENT_TOKEN_RECOVERED_SPAWN_REASON.to_string()
        } else if host_shutdown_recovery_performed || host_shutdown_selection_path.is_file() {
            AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_REASON.to_string()
        } else {
            AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_REASON.to_string()
        },
        runner_program,
        runner_sha256,
        scheduled_pair_count,
        previous_pair_count,
        requested_additional_pair_count,
        added_run_count,
        total_run_count: report.observed_run_count,
        raw_run_count: raw_runs.len(),
        infrastructure_recovery_performed: recovery_performed,
        host_shutdown_recovery_performed,
        recovered_pair_count: if host_shutdown_recovery_performed
            || host_shutdown_selection_path.is_file()
        {
            2
        } else {
            1
        },
        statistically_excluded_run_count: report.statistically_excluded_run_count,
        adjudicated_run_count,
        stopped_early,
        stop_reason,
        claim_eligible: report.claim_eligible,
    };
    write_json_new(&resume_dir.join("result.json"), &result)?;
    Ok(result)
}

/// Names the two lanes of the trailing pair that an authorized infrastructure
/// recovery will replace. Returns an empty set when the prefix cannot be
/// classified, so a malformed campaign still fails revalidation normally
/// rather than silently skipping checks.
fn infrastructure_recovery_exempt_run_ids(
    campaign_dir: &Path,
    schedule: &AgentTokenSchedule,
) -> Result<BTreeSet<String>, String> {
    let runs = crate::load_agent_token_run_summaries(campaign_dir)?;
    Ok(infrastructure_recovery_exempt_run_ids_for(schedule, runs))
}

fn infrastructure_recovery_exempt_run_ids_for(
    schedule: &AgentTokenSchedule,
    runs: Vec<AgentTokenRunSummary>,
) -> BTreeSet<String> {
    let Ok((_, pair_start)) = classify_infrastructure_recovery_prefix(schedule, runs) else {
        return BTreeSet::new();
    };
    schedule
        .entries
        .get(pair_start..pair_start + 2)
        .map(|pair| pair.iter().map(|entry| entry.run_id.clone()).collect())
        .unwrap_or_default()
}

fn classify_infrastructure_recovery_prefix(
    schedule: &AgentTokenSchedule,
    runs: Vec<AgentTokenRunSummary>,
) -> Result<(Vec<AgentTokenRunSummary>, usize), String> {
    let mut by_id = runs
        .into_iter()
        .map(|run| (run.run_id.clone(), run))
        .collect::<BTreeMap<_, _>>();
    let mut ordered = Vec::new();
    let mut encountered_gap = false;
    for entry in &schedule.entries {
        match by_id.remove(&entry.run_id) {
            Some(run) if !encountered_gap => ordered.push(run),
            Some(_) => {
                return Err(format!(
                    "Existing run {} occurs after a missing schedule entry",
                    entry.run_id
                ));
            }
            None => encountered_gap = true,
        }
    }
    if let Some(unexpected) = by_id.keys().next() {
        return Err(format!(
            "Existing run {unexpected} is absent from the frozen schedule"
        ));
    }
    if ordered.is_empty() {
        return Err("Infrastructure recovery requires an observed source lane".to_string());
    }
    let pair_start = if ordered.len().is_multiple_of(2) {
        ordered.len().saturating_sub(2)
    } else {
        ordered.len() - 1
    };
    if !pair_start.is_multiple_of(2) || pair_start + 2 > schedule.entries.len() {
        return Err(
            "Infrastructure recovery source does not end inside one frozen pair".to_string(),
        );
    }
    if let Some(run) = ordered[..pair_start].iter().find(|run| !run.valid_attempt) {
        return Err(format!(
            "Invalid run {} precedes the recoverable pair",
            run.run_id
        ));
    }
    let observed_pair = &ordered[pair_start..];
    if observed_pair.len() > 2
        || !observed_pair.iter().any(|run| {
            run.infrastructure_failure
                .as_deref()
                .is_some_and(recognized_infrastructure_failure)
        })
    {
        return Err(
            "The final observed pair lacks a recognized executor infrastructure failure"
                .to_string(),
        );
    }
    if let Some(run) = observed_pair.iter().find(|run| {
        !run.valid_attempt
            && run
                .infrastructure_failure
                .as_deref()
                .is_none_or(|reason| !recognized_infrastructure_failure(reason))
    }) {
        return Err(format!(
            "Final pair run {} is invalid for a non-infrastructure reason",
            run.run_id
        ));
    }
    Ok((ordered, pair_start))
}

#[allow(clippy::too_many_arguments)]
fn execute_infrastructure_pair_recovery(
    manifest: &AgentTokenCampaignManifest,
    schedule: &AgentTokenSchedule,
    ordered_source_runs: &[AgentTokenRunSummary],
    pair_start: usize,
    campaign_dir: &Path,
    versions: &CapturedVersions,
    runner_program: &Path,
    runner_sha256: &str,
) -> Result<usize, String> {
    let selection_path = campaign_dir.join(AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_SELECTION_FILE);
    if selection_path.exists() {
        return Err(format!(
            "Infrastructure recovery selection already exists: {}",
            selection_path.display()
        ));
    }
    let recovery_root = campaign_dir.join("infrastructure-recoveries/recovery-0001");
    if recovery_root.exists() {
        return Err(format!(
            "Infrastructure recovery evidence already exists; a second attempt is not admitted: {}",
            recovery_root.display()
        ));
    }
    fs::create_dir_all(
        recovery_root
            .parent()
            .expect("infrastructure recovery root has a parent"),
    )
    .map_err(|error| {
        format!(
            "Failed to create infrastructure recovery parent {}: {error}",
            recovery_root.display()
        )
    })?;
    fs::create_dir(&recovery_root).map_err(|error| {
        format!(
            "Failed to create infrastructure recovery root {}: {error}",
            recovery_root.display()
        )
    })?;
    fs::create_dir(recovery_root.join("runs"))
        .map_err(|error| format!("Failed to create infrastructure recovery runs: {error}"))?;
    for file in [
        "campaign-manifest.json",
        "fixture-manifest.json",
        "protocol.json",
    ] {
        copy_file_new(&campaign_dir.join(file), &recovery_root.join(file))?;
    }
    copy_file_new(runner_program, &recovery_root.join("recovery-runner"))?;

    let pair = &schedule.entries[pair_start..pair_start + 2];
    let replacement_entries = pair
        .iter()
        .map(|entry| {
            let mut replacement = entry.clone();
            replacement.run_id = replacement_run_id(&entry.run_id);
            replacement
        })
        .collect::<Vec<_>>();
    write_json_new(
        &recovery_root.join("replacement-pair.json"),
        &serde_json::json!({
            "contract": AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_CONTRACT,
            "policy_revision": AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_POLICY_REVISION,
            "entries": replacement_entries,
        }),
    )?;
    let (preflight, _) = run_executor_preflight(manifest, &recovery_root, versions)?;
    if !preflight.passed {
        write_json_new(
            &recovery_root.join("result.json"),
            &serde_json::json!({
                "contract": AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_EXECUTION_CONTRACT,
                "campaign_id": manifest.campaign_id,
                "admitted": false,
                "preflight_passed": false,
                "failure_reasons": preflight.failure_reasons,
            }),
        )?;
        return Err("Infrastructure recovery executor preflight failed".to_string());
    }

    let (replacement_runs, stop_reason) =
        execute_agent_token_pairs(&replacement_entries, 1, true, |entry| {
            run_one(manifest, entry, &recovery_root, versions)
        })?;
    let replacement_admitted = stop_reason.is_none()
        && replacement_runs.len() == 2
        && replacement_runs.iter().all(|run| {
            run.valid_attempt
                && run.accepted_equivalent
                && run.infrastructure_failure.is_none()
                && run.failure_reasons.is_empty()
        });
    if !replacement_admitted {
        write_json_new(
            &recovery_root.join("result.json"),
            &serde_json::json!({
                "contract": AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_EXECUTION_CONTRACT,
                "campaign_id": manifest.campaign_id,
                "admitted": false,
                "preflight_passed": true,
                "replacement_run_count": replacement_runs.len(),
                "stop_reason": stop_reason,
                "failure_reasons": replacement_runs
                    .iter()
                    .flat_map(|run| run.failure_reasons.iter().chain(run.invalid_reasons.iter()))
                    .cloned()
                    .collect::<Vec<_>>(),
            }),
        )?;
        return Err(
            "Infrastructure replacement pair did not pass every admission gate".to_string(),
        );
    }

    let observed_source_runs = ordered_source_runs[pair_start..]
        .iter()
        .map(|run| {
            recovery_artifact(
                campaign_dir,
                &format!("runs/{}/run-summary.json", run.run_id),
                &run.run_id,
            )
        })
        .collect::<Result<Vec<_>, String>>()?;
    let replacement_artifacts = replacement_runs
        .iter()
        .map(|run| {
            recovery_artifact(
                campaign_dir,
                &format!(
                    "infrastructure-recoveries/recovery-0001/runs/{}/run-summary.json",
                    run.run_id
                ),
                &run.run_id,
            )
        })
        .collect::<Result<Vec<_>, String>>()?;
    let selection = AgentTokenInfrastructurePairRecoverySelection {
        contract: AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        source_protocol_revision: manifest.protocol_revision.clone(),
        policy_revision: AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_POLICY_REVISION.to_string(),
        source_pair_start_index: pair_start,
        workload_id: pair[0].workload_id.clone(),
        attempt: pair[0].attempt,
        source_schedule_run_ids: pair.iter().map(|entry| entry.run_id.clone()).collect(),
        observed_source_runs,
        replacement_runs: replacement_artifacts,
        recovery_runner_sha256: runner_sha256.to_string(),
        reason: AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_REASON.to_string(),
        selected_at: Utc::now().to_rfc3339(),
    };
    crate::agent_token_infrastructure_recovery::validate_selection_identity(
        &selection, manifest, schedule,
    )?;
    write_json_new(&selection_path, &selection)?;
    let view = load_agent_token_campaign_statistical_view(manifest, schedule, campaign_dir)?;
    if view.infrastructure_recovery.is_none()
        || view.effective_runs.len() != pair_start.saturating_add(2)
    {
        return Err(
            "Infrastructure recovery selection did not activate the exact effective prefix"
                .to_string(),
        );
    }
    write_json_new(
        &recovery_root.join("result.json"),
        &serde_json::json!({
            "contract": AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_EXECUTION_CONTRACT,
            "campaign_id": manifest.campaign_id,
            "admitted": true,
            "preflight_passed": true,
            "source_pair_start_index": pair_start,
            "replacement_run_ids": replacement_runs.iter().map(|run| &run.run_id).collect::<Vec<_>>(),
            "recovery_runner_sha256": runner_sha256,
        }),
    )?;
    Ok(2)
}

#[allow(clippy::too_many_arguments)]
fn execute_host_shutdown_pair_recovery(
    manifest: &AgentTokenCampaignManifest,
    schedule: &AgentTokenSchedule,
    effective_prefix_len: usize,
    campaign_dir: &Path,
    versions: &CapturedVersions,
    runner_program: &Path,
    runner_sha256: &str,
) -> Result<usize, String> {
    let selection_path = campaign_dir.join(AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_SELECTION_FILE);
    if selection_path.exists() {
        return Err(format!(
            "Host-shutdown recovery selection already exists: {}",
            selection_path.display()
        ));
    }
    let recovery_root = campaign_dir.join("host-shutdown-recoveries/recovery-0001");
    if recovery_root.exists() {
        return Err(format!(
            "Host-shutdown recovery evidence already exists; a second attempt is not admitted: {}",
            recovery_root.display()
        ));
    }

    let interruption = classify_host_shutdown_interruption(
        manifest,
        schedule,
        campaign_dir,
        effective_prefix_len,
    )?;
    let observation = capture_host_shutdown_observation(interruption.event_mtime_unix_s)?;

    fs::create_dir_all(
        recovery_root
            .parent()
            .expect("host-shutdown recovery root has a parent"),
    )
    .map_err(|error| {
        format!(
            "Failed to create host-shutdown recovery parent {}: {error}",
            recovery_root.display()
        )
    })?;
    fs::create_dir(&recovery_root).map_err(|error| {
        format!(
            "Failed to create host-shutdown recovery root {}: {error}",
            recovery_root.display()
        )
    })?;
    fs::create_dir(recovery_root.join("runs"))
        .map_err(|error| format!("Failed to create host-shutdown recovery runs: {error}"))?;
    for file in [
        "campaign-manifest.json",
        "fixture-manifest.json",
        "protocol.json",
    ] {
        copy_file_new(&campaign_dir.join(file), &recovery_root.join(file))?;
    }
    copy_file_new(runner_program, &recovery_root.join("recovery-runner"))?;
    let observation_path = recovery_root.join("host-shutdown-observation.json");
    write_json_new(&observation_path, &observation)?;

    let pair = &schedule.entries[effective_prefix_len..effective_prefix_len + 2];
    let replacement_entries = pair
        .iter()
        .map(|entry| {
            let mut replacement = entry.clone();
            replacement.run_id = host_shutdown_replacement_run_id(&entry.run_id);
            replacement
        })
        .collect::<Vec<_>>();
    write_json_new(
        &recovery_root.join("replacement-pair.json"),
        &serde_json::json!({
            "contract": AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_CONTRACT,
            "policy_revision": AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_POLICY_REVISION,
            "entries": replacement_entries,
        }),
    )?;
    let (preflight, _) = run_executor_preflight(manifest, &recovery_root, versions)?;
    if !preflight.passed {
        write_json_new(
            &recovery_root.join("result.json"),
            &serde_json::json!({
                "contract": AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_EXECUTION_CONTRACT,
                "campaign_id": manifest.campaign_id,
                "admitted": false,
                "preflight_passed": false,
                "failure_reasons": preflight.failure_reasons,
            }),
        )?;
        return Err("Host-shutdown recovery executor preflight failed".to_string());
    }

    let (replacement_runs, stop_reason) =
        execute_agent_token_pairs(&replacement_entries, 1, true, |entry| {
            run_one(manifest, entry, &recovery_root, versions)
        })?;
    let replacement_admitted = stop_reason.is_none()
        && replacement_runs.len() == 2
        && replacement_runs.iter().all(|run| {
            run.valid_attempt
                && run.infrastructure_failure.is_none()
                && run.invalid_reasons.is_empty()
                && run.usage.is_some()
                && run.transcript.valid
                && run.transcript.errors.is_empty()
        });
    if !replacement_admitted {
        write_json_new(
            &recovery_root.join("result.json"),
            &serde_json::json!({
                "contract": AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_EXECUTION_CONTRACT,
                "campaign_id": manifest.campaign_id,
                "admitted": false,
                "preflight_passed": true,
                "replacement_run_count": replacement_runs.len(),
                "stop_reason": stop_reason,
                "failure_reasons": replacement_runs
                    .iter()
                    .flat_map(|run| run.failure_reasons.iter().chain(run.invalid_reasons.iter()))
                    .cloned()
                    .collect::<Vec<_>>(),
            }),
        )?;
        return Err(
            "Host-shutdown replacement pair did not complete as two protocol-valid lanes"
                .to_string(),
        );
    }

    let replacement_artifacts = replacement_runs
        .iter()
        .map(|run| {
            recovery_artifact(
                campaign_dir,
                &format!(
                    "host-shutdown-recoveries/recovery-0001/runs/{}/run-summary.json",
                    run.run_id
                ),
                &run.run_id,
            )
        })
        .collect::<Result<Vec<_>, String>>()?;
    let observation_relative =
        "host-shutdown-recoveries/recovery-0001/host-shutdown-observation.json";
    let observation_bytes = fs::read(&observation_path).map_err(|error| {
        format!(
            "Failed to read host-shutdown observation {}: {error}",
            observation_path.display()
        )
    })?;
    let selection = AgentTokenHostShutdownPairRecoverySelection {
        contract: AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        source_protocol_revision: manifest.protocol_revision.clone(),
        policy_revision: AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_POLICY_REVISION.to_string(),
        source_pair_start_index: effective_prefix_len,
        workload_id: pair[0].workload_id.clone(),
        attempt: pair[0].attempt,
        source_schedule_run_ids: pair.iter().map(|entry| entry.run_id.clone()).collect(),
        interrupted_run_id: pair[0].run_id.clone(),
        interrupted_run_directory: format!("runs/{}", pair[0].run_id),
        interrupted_artifacts: interruption.artifacts,
        interrupted_event_path: format!("runs/{}/private/codex-events.raw.jsonl", pair[0].run_id),
        interrupted_event_sha256: interruption.event_sha256,
        interrupted_event_mtime_unix_s: interruption.event_mtime_unix_s,
        terminal_provider_event_observed: false,
        run_summary_observed: false,
        host_observation: observation_relative.to_string(),
        host_observation_sha256: sha256_digest(&observation_bytes),
        replacement_runs: replacement_artifacts,
        recovery_runner_sha256: runner_sha256.to_string(),
        reason: AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_REASON.to_string(),
        selected_at: Utc::now().to_rfc3339(),
    };
    crate::agent_token_host_shutdown_recovery::validate_host_shutdown_selection_identity(
        &selection, manifest, schedule,
    )?;
    write_json_new(&selection_path, &selection)?;
    let view = load_agent_token_campaign_statistical_view(manifest, schedule, campaign_dir)?;
    if view.host_shutdown_recovery.is_none()
        || view.effective_runs.len() != effective_prefix_len.saturating_add(2)
    {
        return Err(
            "Host-shutdown recovery selection did not activate the exact effective prefix"
                .to_string(),
        );
    }
    write_json_new(
        &recovery_root.join("result.json"),
        &serde_json::json!({
            "contract": AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_EXECUTION_CONTRACT,
            "campaign_id": manifest.campaign_id,
            "admitted": true,
            "preflight_passed": true,
            "source_pair_start_index": effective_prefix_len,
            "replacement_run_ids": replacement_runs.iter().map(|run| &run.run_id).collect::<Vec<_>>(),
            "recovery_runner_sha256": runner_sha256,
        }),
    )?;
    Ok(2)
}

fn recovery_artifact(
    campaign_dir: &Path,
    relative: &str,
    run_id: &str,
) -> Result<AgentTokenInfrastructureRecoveryArtifact, String> {
    let path = campaign_dir.join(relative);
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "Failed to read infrastructure recovery artifact {}: {error}",
            path.display()
        )
    })?;
    Ok(AgentTokenInfrastructureRecoveryArtifact {
        run_id: run_id.to_string(),
        run_summary: relative.to_string(),
        run_summary_sha256: sha256_digest(&bytes),
    })
}

fn validate_resume_schedule(
    manifest: &AgentTokenCampaignManifest,
    observed: &AgentTokenSchedule,
) -> Result<(), String> {
    let expected = build_agent_token_schedule(manifest);
    let observed_value = serde_json::to_value(observed)
        .map_err(|error| format!("Failed to normalize stored campaign schedule: {error}"))?;
    let expected_value = serde_json::to_value(expected)
        .map_err(|error| format!("Failed to normalize expected campaign schedule: {error}"))?;
    if observed_value != expected_value {
        return Err("Stored campaign schedule differs from its immutable manifest".to_string());
    }
    Ok(())
}

fn exact_schedule_prefix(
    schedule: &AgentTokenSchedule,
    runs: Vec<AgentTokenRunSummary>,
) -> Result<Vec<AgentTokenRunSummary>, String> {
    let mut by_id = BTreeMap::new();
    for run in runs {
        let run_id = run.run_id.clone();
        if by_id.insert(run_id.clone(), run).is_some() {
            return Err(format!("Duplicate existing run summary id: {run_id}"));
        }
    }
    let mut ordered = Vec::new();
    let mut encountered_gap = false;
    for entry in &schedule.entries {
        match by_id.remove(&entry.run_id) {
            Some(run) if !encountered_gap => ordered.push(run),
            Some(_) => {
                return Err(format!(
                    "Existing run {} occurs after a missing schedule entry",
                    entry.run_id
                ));
            }
            None => encountered_gap = true,
        }
    }
    if let Some(unexpected) = by_id.keys().next() {
        return Err(format!(
            "Existing run {unexpected} is absent from the frozen schedule"
        ));
    }
    if !ordered.len().is_multiple_of(2) {
        return Err(format!(
            "Existing campaign ends with a partial pair after {} runs",
            ordered.len()
        ));
    }
    Ok(ordered)
}

fn append_supported_run_adjudications(
    campaign_dir: &Path,
    manifest: &AgentTokenCampaignManifest,
    runs: &[AgentTokenRunSummary],
) -> Result<(), String> {
    for run in runs.iter().filter(|run| !run.valid_attempt) {
        let path = campaign_dir
            .join("adjudications")
            .join(format!("{}.json", run.run_id));
        if path.exists() {
            continue;
        }
        let adjudication =
            build_agent_token_run_adjudication(campaign_dir, run, &manifest.protocol_revision)?;
        write_json_new(&path, &adjudication)?;
    }
    Ok(())
}

fn append_recovered_spawn_adjudication(
    campaign_dir: &Path,
    manifest: &AgentTokenCampaignManifest,
) -> Result<(), String> {
    if manifest.campaign_id != AGENT_TOKEN_RECOVERED_SPAWN_CAMPAIGN_ID
        || manifest.protocol_revision
            != crate::AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION
    {
        return Err(
            "Recovered-spawn adjudication is not authorized for this campaign or source protocol"
                .to_string(),
        );
    }
    let adjudication_path = campaign_dir
        .join("adjudications")
        .join(format!("{AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID}.json"));
    if adjudication_path.exists() {
        return Err(format!(
            "Recovered-spawn adjudication already exists: {}",
            adjudication_path.display()
        ));
    }
    let source = decode_json_file::<AgentTokenRunSummary>(
        &campaign_dir
            .join("runs")
            .join(AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID)
            .join("run-summary.json"),
        "recovered-spawn source run summary",
    )?;
    let adjudication =
        build_agent_token_run_adjudication(campaign_dir, &source, &manifest.protocol_revision)?;
    write_json_new(&adjudication_path, &adjudication)
}

fn require_resume_version_identity(
    campaign_dir: &Path,
    runs: &[AgentTokenRunSummary],
    current: &CapturedVersions,
) -> Result<(), String> {
    let first = runs
        .first()
        .ok_or_else(|| "Campaign resume requires at least one completed pair".to_string())?;
    let environment = decode_json_file::<AgentTokenEnvironment>(
        &campaign_dir
            .join("runs")
            .join(&first.run_id)
            .join("environment.json"),
        "existing run environment",
    )?;
    for run in runs.iter().skip(1) {
        let observed = decode_json_file::<AgentTokenEnvironment>(
            &campaign_dir
                .join("runs")
                .join(&run.run_id)
                .join("environment.json"),
            "existing run environment",
        )?;
        if observed.codex_version != environment.codex_version
            || observed.ait_version != environment.ait_version
            || observed.git_version != environment.git_version
            || observed.node_version != environment.node_version
            || observed.browser_version != environment.browser_version
        {
            return Err(format!(
                "Existing run {} tool versions differ from the frozen campaign prefix",
                run.run_id
            ));
        }
    }
    let observed = [
        (
            "Codex",
            environment.codex_version.as_str(),
            current.codex.as_str(),
        ),
        (
            "AIT",
            environment.ait_version.as_str(),
            current.ait.as_str(),
        ),
        (
            "Git",
            environment.git_version.as_str(),
            current.git.as_str(),
        ),
        (
            "Node",
            environment.node_version.as_str(),
            current.node.as_str(),
        ),
    ];
    for (name, expected, actual) in observed {
        if expected != actual {
            return Err(format!(
                "Campaign resume {name} version drifted: expected {expected:?}, got {actual:?}"
            ));
        }
    }
    if environment.browser_version != current.browser {
        return Err(format!(
            "Campaign resume browser version drifted: expected {:?}, got {:?}",
            environment.browser_version, current.browser
        ));
    }
    Ok(())
}

fn next_resume_directory(campaign_dir: &Path) -> Result<PathBuf, String> {
    let root = campaign_dir.join("resumptions");
    fs::create_dir_all(&root)
        .map_err(|error| format!("Failed to create {}: {error}", root.display()))?;
    for ordinal in 1..=9_999 {
        let path = root.join(format!("resume-{ordinal:04}"));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Failed to create campaign resume directory {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Err("Campaign resume ordinal space is exhausted".to_string())
}

pub fn enrich_agent_token_report_model_composition(
    manifest: &AgentTokenCampaignManifest,
    runs: &[AgentTokenRunSummary],
    run_summary_paths: &BTreeMap<String, PathBuf>,
    report: &mut AgentTokenReport,
) -> Result<(), String> {
    report.served_models.clear();
    report.mixed_model_run_count = 0;
    report.fallback_observed_run_count = 0;
    if manifest.runtime.executor != crate::agent_token::AgentTokenExecutor::Claude
        || !protocol_requires_claude_model_evidence(&manifest.protocol_revision)
    {
        return Ok(());
    }

    let mut totals = BTreeMap::<(String, String), AgentTokenServedModelReport>::new();
    for run in runs.iter().filter(|run| run.usage.is_some()) {
        let summary_path = run_summary_paths.get(&run.run_id).ok_or_else(|| {
            format!(
                "Effective run {} has no summary path for model-composition reporting",
                run.run_id
            )
        })?;
        let run_dir = summary_path.parent().ok_or_else(|| {
            format!(
                "Effective run summary has no parent directory: {}",
                summary_path.display()
            )
        })?;
        let imported = crate::agent_token::import_claude_usage_with_outcome(
            &run_dir.join("private/codex-events.raw.jsonl"),
            &run.run_id,
            &run.workload_id,
            run.mode,
            run.accounting_profile,
            &manifest.model,
            manifest.claude_model_admission,
        )?;
        if run.usage.as_ref() != Some(&imported.usage) {
            return Err(format!(
                "Run {} model-composition source differs from normalized usage",
                run.run_id
            ));
        }
        report.mixed_model_run_count += usize::from(imported.served_models.len() > 1);
        report.fallback_observed_run_count += usize::from(imported.fallback_observed);
        for served in imported.served_models {
            let total = totals
                .entry((served.model_id.clone(), served.canonical_model.clone()))
                .or_insert_with(|| AgentTokenServedModelReport {
                    model_id: served.model_id.clone(),
                    canonical_model: served.canonical_model.clone(),
                    ..AgentTokenServedModelReport::default()
                });
            total.run_count = total.run_count.saturating_add(1);
            total.input_tokens = checked_model_report_sum(
                total.input_tokens,
                served.input_tokens,
                &served.model_id,
            )?;
            total.cached_input_tokens = checked_model_report_sum(
                total.cached_input_tokens,
                served.cached_input_tokens,
                &served.model_id,
            )?;
            total.cache_write_input_tokens = checked_model_report_sum(
                total.cache_write_input_tokens,
                served.cache_write_input_tokens,
                &served.model_id,
            )?;
            total.output_tokens = checked_model_report_sum(
                total.output_tokens,
                served.output_tokens,
                &served.model_id,
            )?;
            total.provider_total_tokens = checked_model_report_sum(
                total.provider_total_tokens,
                served.provider_total_tokens,
                &served.model_id,
            )?;
        }
    }
    report.served_models = totals.into_values().collect();
    Ok(())
}

fn checked_model_report_sum(current: u64, added: u64, model_id: &str) -> Result<u64, String> {
    current
        .checked_add(added)
        .ok_or_else(|| format!("Served-model token total overflowed u64 for model {model_id}"))
}

fn refresh_campaign_derived_views(
    manifest: &AgentTokenCampaignManifest,
    schedule: &AgentTokenSchedule,
    runs: &[AgentTokenRunSummary],
    campaign_dir: &Path,
) -> Result<crate::AgentTokenReport, String> {
    let preflight = decode_json_file::<AgentTokenExecutorPreflightReport>(
        &campaign_dir.join("executor-preflight-report.json"),
        "executor preflight report",
    )?;
    let git_permission_preflight =
        decode_json_file::<AgentTokenGitWorktreePermissionPreflightReport>(
            &campaign_dir.join("git-worktree-permission-preflight-report.json"),
            "Git worktree permission preflight report",
        )?;
    let index = AgentTokenRunIndex {
        contract: AGENT_TOKEN_RUN_INDEX_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        generated_at: Utc::now().to_rfc3339(),
        scheduled_run_count: schedule.entries.len(),
        executed_run_count: runs.len(),
        runs: runs
            .iter()
            .map(|run| AgentTokenRunIndexEntry {
                run_id: run.run_id.clone(),
                workload_id: run.workload_id.clone(),
                mode: run.mode,
                attempt: run.attempt,
                valid_attempt: run.valid_attempt,
                accepted_equivalent: run.accepted_equivalent,
                provider_total_tokens: run.usage.as_ref().map(|usage| usage.provider_total_tokens),
                run_summary: format!("runs/{}/run-summary.json", run.run_id),
                adjudication: campaign_dir
                    .join("adjudications")
                    .join(format!("{}.json", run.run_id))
                    .exists()
                    .then(|| format!("adjudications/{}.json", run.run_id)),
            })
            .collect(),
    };
    write_json_derived(&campaign_dir.join("raw-run-index.json"), &index)?;
    let statistical_view =
        load_agent_token_campaign_statistical_view(manifest, schedule, campaign_dir)?;
    let mut report = statistical_view.report;
    enrich_agent_token_report_model_composition(
        manifest,
        &statistical_view.effective_runs,
        &statistical_view.effective_run_summary_paths,
        &mut report,
    )?;
    if statistical_view.selection.is_some() || statistical_view.infrastructure_recovery.is_some() {
        let effective_index = AgentTokenRunIndex {
            contract: AGENT_TOKEN_RUN_INDEX_CONTRACT.to_string(),
            campaign_id: manifest.campaign_id.clone(),
            generated_at: Utc::now().to_rfc3339(),
            scheduled_run_count: statistical_view.effective_schedule.entries.len(),
            executed_run_count: statistical_view.effective_runs.len(),
            runs: statistical_view
                .effective_runs
                .iter()
                .map(|run| {
                    let path = statistical_view
                        .effective_run_summary_paths
                        .get(&run.run_id)
                        .expect("effective run has a summary path");
                    let relative = path.strip_prefix(campaign_dir).map_err(|_| {
                        format!(
                            "Effective run summary escaped the campaign root: {}",
                            path.display()
                        )
                    })?;
                    Ok(AgentTokenRunIndexEntry {
                        run_id: run.run_id.clone(),
                        workload_id: run.workload_id.clone(),
                        mode: run.mode,
                        attempt: run.attempt,
                        valid_attempt: run.valid_attempt,
                        accepted_equivalent: run.accepted_equivalent,
                        provider_total_tokens: run
                            .usage
                            .as_ref()
                            .map(|usage| usage.provider_total_tokens),
                        run_summary: relative.display().to_string(),
                        adjudication: campaign_dir
                            .join("adjudications")
                            .join(format!("{}.json", run.run_id))
                            .is_file()
                            .then(|| format!("adjudications/{}.json", run.run_id)),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        };
        write_json_derived(
            &campaign_dir.join("effective-run-index.json"),
            &effective_index,
        )?;
    }
    write_json_derived(&campaign_dir.join("aggregate-report.json"), &report)?;
    write_json_derived(
        &campaign_dir.join("comparison-report.json"),
        &serde_json::json!({
            "contract": "ait-agent-token-mode-comparison-report/v3",
            "campaign_id": report.campaign_id,
            "protocol_revision": report.protocol_revision,
            "campaign_scope": report.campaign_scope,
            "ait_edit_root_mode": report.ait_edit_root_mode,
            "git_worktree_mode": report.git_worktree_mode,
            "git_worktree_permission_preflight_passed": git_permission_preflight.passed,
            "executor_preflight_passed": preflight.passed,
            "source_protocol_claim_eligible": report.source_protocol_claim_eligible,
            "current_policy_revision": report.current_policy_revision,
            "current_policy_evaluation_mode": report.current_policy_evaluation_mode,
            "current_policy_criteria_met": report.current_policy_criteria_met,
            "claim_eligible": report.claim_eligible,
            "served_models": report.served_models,
            "mixed_model_run_count": report.mixed_model_run_count,
            "fallback_observed_run_count": report.fallback_observed_run_count,
            "executed_evidence_run_count": report.executed_evidence_run_count,
            "statistically_excluded_run_count": report.statistically_excluded_run_count,
            "replacement_policy_revision": report.replacement_policy_revision,
            "statistical_replacements": report.statistical_replacements,
            "infrastructure_recovery_policy_revision": report.infrastructure_recovery_policy_revision,
            "infrastructure_pair_recoveries": report.infrastructure_pair_recoveries,
            "host_shutdown_recovery_policy_revision": report.host_shutdown_recovery_policy_revision,
            "host_shutdown_pair_recoveries": report.host_shutdown_pair_recoveries,
            "recovered_spawn_policy_revision": report.recovered_spawn_policy_revision,
            "recovered_spawn_adjudications": report.recovered_spawn_adjudications,
            "pair_admission_policy": report.pair_admission_policy,
            "comparisons": report.comparisons,
            "blockers": report.blockers,
            "source_protocol_blockers": report.source_protocol_blockers,
            "current_policy_blockers": report.current_policy_blockers,
        }),
    )?;
    let mut claim_boundary = render_agent_token_report_markdown(&report);
    if report.replacement_policy_revision.is_some()
        && report.recovered_spawn_policy_revision.is_some()
    {
        claim_boundary.push_str(&format!(
            "\n## Claim Boundary\n\nThe executor-infrastructure and host-shutdown whole-pair recoveries remain separately disclosed and immutable. Raw `b017-gd-03-git` remains byte-for-byte unchanged under its digest-linked recovered-spawn adjudication. The original valid-but-unaccepted GD-05 AIT lane also remains checksummed and disclosed; one exact same-pinned AIT lane was executed once under the repository-owner-authorized statistical-replacement policy and admitted only after passing every original gate. Effective claim eligibility is derived from exactly 200 admitted sessions selected from {} executed evidence sessions with {} exclusions. This exact authorization does not establish a general retry policy or erase the functional failure. The finding remains limited to the pinned game-development workloads, model, accounting profile, and single-session local topology.\n",
            report.executed_evidence_run_count,
            report.statistically_excluded_run_count,
        ));
    } else if report.recovered_spawn_policy_revision.is_some() {
        claim_boundary.push_str(
            "\n## Claim Boundary\n\nThe prior executor-infrastructure and host-shutdown recoveries remain separately disclosed and immutable. Raw `b017-gd-03-git` also remains byte-for-byte unchanged with its original spawn-failure classification; a SHA-256-linked successor adjudication admits the same completed lane because the session recovered, exited zero, retained valid transcript and provider usage, and reached normal evaluation. Its retry and all tokens remain measured, and no successful Git lane was re-executed. Effective claim eligibility requires exactly 200 admitted sessions and separately reports all 202 executed sessions and both exclusions. Functional candidate outcomes remain measured and never authorize retry. The finding remains limited to the pinned game-development workloads, model, accounting profile, and single-session local topology.\n",
        );
    } else if report.host_shutdown_recovery_policy_revision.is_some() {
        claim_boundary.push_str(
            "\n## Claim Boundary\n\nThe 2026-08-29 host shutdown interrupted one Git lane without a terminal provider event or run summary. The partial directory remains byte-for-byte in place with a checksummed inventory and host observation, and its whole Git/AIT pair was re-executed exactly once under distinct run IDs before continuing the unchanged frozen suffix. The earlier recognized executor-infrastructure recovery remains separately disclosed. Effective claim eligibility requires exactly 200 admitted sessions and separately reports all 202 executed sessions and both exclusions. Functional candidate outcomes remain measured and never authorize retry. The finding remains limited to the pinned game-development workloads, model, accounting profile, and single-session local topology.\n",
        );
    } else if report.infrastructure_recovery_policy_revision.is_some() {
        claim_boundary.push_str(
            "\n## Claim Boundary\n\nThe recognized executor infrastructure failure and every observed source lane remain immutable in append-only evidence. The contaminated Git/AIT pair is excluded in full, and both same-pinned lanes are re-executed exactly once under the disclosed whole-pair recovery policy. Effective claim eligibility requires exactly 200 admitted sessions and separately reports all executed and excluded evidence. Functional failures, evaluator failures, and workflow failures are never retryable under this policy. The finding remains limited to the pinned game-development workloads, model, accounting profile, and single-session local topology.\n",
        );
    } else if report.replacement_policy_revision.is_some() {
        claim_boundary.push_str(
            "\n## Claim Boundary\n\nThe frozen source-protocol result and its disclosed GD-05 AIT failure remain immutable. Effective claim eligibility is derived under the repository-owner-authorized transparent replacement policy from exactly 200 statistically admitted sessions selected from 201 executed evidence sessions. The finding remains limited to the pinned game-development workloads, model, accounting profile, and single-session local topology. It does not connect to `ait-server`, establish a high-concurrency result, or support a universal AIT-versus-Git product claim.\n",
        );
    } else {
        claim_boundary.push_str(
            "\n## Claim Boundary\n\nSource-protocol claim eligibility is authoritative. A retrospective current-policy evaluation never changes the source campaign scope or makes an ineligible source campaign protocol-qualified. This campaign compares only the pinned game-development workloads, model, accounting profile, and single-session local topology. Its executor-preflight tokens are admission overhead and are excluded from AIT/Git metrics. It does not connect to `ait-server` and does not support a general AIT-versus-Git product claim.\n",
        );
    }
    write_text_derived(&campaign_dir.join("claim-boundary.md"), &claim_boundary)?;
    Ok(report)
}

fn write_json_derived(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Failed to encode derived JSON {}: {error}", path.display()))?;
    bytes.push(b'\n');
    write_bytes_derived(path, &bytes)
}

fn write_text_derived(path: &Path, value: &str) -> Result<(), String> {
    write_bytes_derived(path, value.as_bytes())
}

fn write_bytes_derived(path: &Path, bytes: &[u8]) -> Result<(), String> {
    ensure_parent(path)?;
    if path.exists() {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "Derived report path is not a regular file: {}",
                path.display()
            ));
        }
        let existing = fs::read(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        if existing == bytes {
            return Ok(());
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                format!(
                    "Derived report has no portable file name: {}",
                    path.display()
                )
            })?;
        let archive = path
            .parent()
            .expect("derived report has parent")
            .join("derived-history")
            .join(format!(
                "{file_name}.{}",
                sha256_digest(&existing).trim_start_matches("sha256:")
            ));
        if !archive.exists() {
            ensure_parent(&archive)?;
            let mut output = create_new_file(&archive)?;
            output.write_all(&existing).map_err(|error| {
                format!(
                    "Failed to archive derived report {}: {error}",
                    archive.display()
                )
            })?;
        }
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Derived report has no file name: {}", path.display()))?;
    let temporary = path
        .parent()
        .expect("derived report has parent")
        .join(format!(
            ".{file_name}.resume-{}-{}.tmp",
            std::process::id(),
            Utc::now().timestamp_micros()
        ));
    let mut output = create_new_file(&temporary)?;
    output.write_all(bytes).map_err(|error| {
        format!(
            "Failed to write derived report temporary file {}: {error}",
            temporary.display()
        )
    })?;
    drop(output);
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(first_error) if path.exists() => {
            fs::remove_file(path).map_err(|error| {
                format!(
                    "Failed to replace archived derived report {} after {first_error}: {error}",
                    path.display()
                )
            })?;
            fs::rename(&temporary, path).map_err(|error| {
                format!(
                    "Failed to activate derived report {}: {error}",
                    path.display()
                )
            })
        }
        Err(error) => Err(format!(
            "Failed to activate derived report {}: {error}",
            path.display()
        )),
    }
}

fn run_one(
    manifest: &AgentTokenCampaignManifest,
    entry: &AgentTokenScheduleEntry,
    campaign_dir: &Path,
    versions: &CapturedVersions,
) -> Result<AgentTokenRunSummary, String> {
    let run_dir = campaign_dir.join("runs").join(&entry.run_id);
    prepare_empty_directory(&run_dir, "run evidence")?;
    let workspace = run_dir.join("workspace");
    let receipt = materialize_game_fixture(
        &manifest.runtime.fixture_manifest,
        &entry.workload_id,
        &workspace,
    )?;
    copy_file_new(
        &campaign_dir.join("campaign-manifest.json"),
        &run_dir.join("campaign-manifest.json"),
    )?;
    copy_file_new(
        &campaign_dir.join("fixture-manifest.json"),
        &run_dir.join("fixture-manifest.json"),
    )?;
    let git_task_worktree_container = (entry.mode == AgentTokenMode::GitLinearSingleSession)
        .then(|| run_dir.join("git-worktree-runtime"));
    let git_task_worktree_path = git_task_worktree_container
        .as_ref()
        .map(|container| container.join("git-task-worktree"));
    let git_metadata_path = (entry.mode == AgentTokenMode::GitLinearSingleSession)
        .then(|| run_dir.join("private/git-metadata"));
    // An explicit AIT treatment owns a benchmark-supplied path. A returned
    // treatment deliberately creates no such path: task start chooses the
    // worktree and the measured agent follows its next_action command.
    let ait_task_worktree_container = (entry.mode == AgentTokenMode::AitLinearSingleSession
        && manifest.ait_edit_root_mode == crate::agent_token::AgentTokenAitEditRootMode::Explicit)
        .then(|| run_dir.join("ait-worktree-runtime"));
    let ait_task_worktree_path = ait_task_worktree_container
        .as_ref()
        .map(|container| container.join("ait-task-worktree"));
    if let Some(path) = git_task_worktree_container.as_deref() {
        prepare_empty_directory(path, "Git task worktree container")?;
    }
    if let Some(path) = git_metadata_path.as_deref() {
        prepare_empty_directory(path, "Git metadata")?;
    }
    if let Some(path) = ait_task_worktree_container.as_deref() {
        prepare_empty_directory(path, "AIT task worktree container")?;
    }

    let shared_task = fs::read_to_string(workspace.join("TASK.txt")).map_err(|error| {
        format!(
            "Failed to read workload task {}: {error}",
            workspace.join("TASK.txt").display()
        )
    })?;
    let prompt = build_measured_prompt(
        manifest,
        entry,
        &shared_task,
        git_task_worktree_path.as_deref(),
        git_metadata_path.as_deref(),
        ait_task_worktree_path.as_deref(),
    );
    write_text_new(&run_dir.join("prompt.txt"), &prompt)?;
    let shared_task_prompt_digest = sha256_digest(shared_task.as_bytes());
    let measured_prompt_digest = sha256_digest(prompt.as_bytes());

    let mut bootstrap_events = Vec::new();
    let mut sequence = 1_usize;
    let (add_dirs, git_worktree_path, git_write_exceptions) =
        match (manifest.accounting_profile, entry.mode) {
            (
                crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
                AgentTokenMode::GitLinearSingleSession,
            ) => {
                let metadata = git_metadata_path
                    .clone()
                    .expect("Git mode prepared its metadata path");
                bootstrap_git(
                    manifest,
                    &workspace,
                    &metadata,
                    &mut bootstrap_events,
                    &mut sequence,
                )?;
                let path = git_task_worktree_path
                    .clone()
                    .expect("Git mode prepared its linked-worktree path");
                let container = git_task_worktree_container
                    .clone()
                    .expect("Git mode prepared its linked-worktree container");
                (
                    vec![metadata.clone(), container],
                    Some(path.clone()),
                    git_write_exceptions_for_mode(
                        manifest.git_worktree_mode,
                        &workspace,
                        &metadata,
                        &path,
                    ),
                )
            }
            (
                crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
                AgentTokenMode::AitLinearSingleSession,
            ) => {
                let ephemeral =
                    bootstrap_ait(manifest, &workspace, &mut bootstrap_events, &mut sequence)?
                        .worktree_add_dir;
                let mut writable_roots = vec![ephemeral];
                if let Some(container) = ait_task_worktree_container.clone() {
                    writable_roots.push(container);
                }
                (writable_roots, None, Vec::new())
            }
            (
                crate::AgentTokenAccountingProfile::FirstUseTotalCost,
                AgentTokenMode::AitLinearSingleSession,
            ) => (
                manifest
                    .runtime
                    .ait_first_use_worktree_add_dir
                    .iter()
                    .cloned()
                    .collect(),
                None,
                Vec::new(),
            ),
            (
                crate::AgentTokenAccountingProfile::FirstUseTotalCost,
                AgentTokenMode::GitLinearSingleSession,
            ) => {
                let metadata = git_metadata_path
                    .clone()
                    .expect("Git mode prepared its metadata path");
                let path = git_task_worktree_path
                    .clone()
                    .expect("Git mode prepared its linked-worktree path");
                let container = git_task_worktree_container
                    .clone()
                    .expect("Git mode prepared its linked-worktree container");
                (
                    vec![metadata.clone(), container],
                    Some(path.clone()),
                    git_write_exceptions_for_mode(
                        manifest.git_worktree_mode,
                        &workspace,
                        &metadata,
                        &path,
                    ),
                )
            }
        };
    let git_start_state_proof =
        (protocol_requires_git_start_state_proof(&manifest.protocol_revision)
            && manifest.accounting_profile == AgentTokenAccountingProfile::SteadyStateTaskCost
            && entry.mode == AgentTokenMode::GitLinearSingleSession)
            .then(|| capture_git_start_state_proof(manifest, &entry.run_id, &workspace));
    if let Some(proof) = git_start_state_proof.as_ref() {
        write_json_new(&run_dir.join("git-start-state-proof.json"), proof)?;
        if !proof.passed {
            return Err(format!(
                "Git start-state proof failed for {}: {}",
                entry.run_id,
                proof.failure_reasons.join("; ")
            ));
        }
    }
    let mut managed_worktree_lifecycle = if entry.mode == AgentTokenMode::GitLinearSingleSession
        && manifest.git_worktree_mode == AgentTokenGitWorktreeMode::CodexAppEquivalentManaged
    {
        let worktree = git_task_worktree_path
            .as_deref()
            .expect("managed Git mode prepared its worktree path");
        let starting_head = git_start_state_proof
            .as_ref()
            .and_then(|proof| proof.head_oid.as_deref())
            .ok_or_else(|| {
                "Managed Git worktree provisioning requires a proven starting HEAD".to_string()
            })?;
        Some(provision_managed_git_worktree(
            manifest,
            entry,
            &workspace,
            worktree,
            starting_head,
            &mut bootstrap_events,
            &mut sequence,
        )?)
    } else {
        None
    };
    let execution_workspace = if managed_worktree_lifecycle.is_some() {
        git_task_worktree_path
            .as_deref()
            .expect("managed Git mode prepared its execution worktree")
    } else {
        workspace.as_path()
    };
    let permission_profile =
        build_codex_permission_profile(execution_workspace, &add_dirs, &git_write_exceptions)?;
    write_json_new(
        &run_dir.join("codex-permission-profile.json"),
        &permission_profile,
    )?;
    write_json_lines_new(
        &run_dir.join("private/bootstrap-events.jsonl"),
        &bootstrap_events,
    )?;

    let run_manifest = AgentTokenRunManifest {
        contract: AGENT_TOKEN_RUN_MANIFEST_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        run_id: entry.run_id.clone(),
        workload_id: entry.workload_id.clone(),
        mode: entry.mode,
        accounting_profile: manifest.accounting_profile.as_str().to_string(),
        attempt: entry.attempt,
        block_index: entry.block_index,
        randomized_order: entry.randomized_order,
        fixture_revision: receipt.fixture_revision.clone(),
        fixture_content_digest: receipt.content_digest.clone(),
        shared_task_prompt_digest,
        measured_prompt_digest,
        workspace: "workspace".to_string(),
        execution_workspace: execution_workspace
            .strip_prefix(&run_dir)
            .unwrap_or(execution_workspace)
            .display()
            .to_string(),
        ait_edit_root_mode: manifest.ait_edit_root_mode,
        git_worktree_mode: manifest.git_worktree_mode,
        network_policy: manifest.network_policy.clone(),
        tool_policy: manifest.tool_policy.clone(),
        codex_permission_profile: permission_profile.name.clone(),
        codex_permission_profile_parent: permission_profile.extends.clone(),
        benchmark_enabled_feature_overrides: executor_enabled_feature_overrides(manifest),
        benchmark_disabled_feature_overrides: executor_disabled_feature_overrides(manifest),
        project_document_loading: project_document_loading_label(
            manifest.runtime.project_doc_max_bytes,
        ),
        project_doc_max_bytes: manifest.runtime.project_doc_max_bytes,
        workflow_mode: match entry.mode {
            AgentTokenMode::GitLinearSingleSession => {
                git_workflow_mode_label(manifest.git_worktree_mode).to_string()
            }
            AgentTokenMode::AitLinearSingleSession => "solo_local".to_string(),
        },
        sprint_mode: match entry.mode {
            AgentTokenMode::GitLinearSingleSession => "not_applicable".to_string(),
            AgentTokenMode::AitLinearSingleSession => manifest.ait_sprint_mode.as_str().to_string(),
        },
        sprint_card_path: (entry.mode == AgentTokenMode::AitLinearSingleSession
            && manifest.ait_sprint_mode == AgentTokenAitSprintMode::On)
            .then(|| AIT_SPRINT_CARD_PATH.to_string()),
        sprint_item_ref: (entry.mode == AgentTokenMode::AitLinearSingleSession
            && manifest.ait_sprint_mode == AgentTokenAitSprintMode::On)
            .then(|| sprint_item_ref(&entry.run_id)),
        ait_server_allowed: false,
        git_start_state_proof: git_start_state_proof
            .as_ref()
            .map(|_| "git-start-state-proof.json".to_string()),
    };
    write_json_new(&run_dir.join("run-manifest.json"), &run_manifest)?;

    let raw_events = run_dir.join("private/codex-events.raw.jsonl");
    let codex_stderr = run_dir.join("private/codex.stderr.txt");
    if let Some(lifecycle) = managed_worktree_lifecycle.as_mut() {
        lifecycle.model_started_at = Some(Utc::now().to_rfc3339());
    }
    let codex_result = run_measured_agent(
        manifest,
        execution_workspace,
        &add_dirs,
        &git_write_exceptions,
        &prompt,
        &raw_events,
        &codex_stderr,
    );
    let model_request_state_result =
        inspect_model_request_state(manifest, &raw_events, &entry.run_id);
    if let Some(lifecycle) = managed_worktree_lifecycle.as_mut() {
        lifecycle.model_completed_at = Some(Utc::now().to_rfc3339());
        lifecycle.model_elapsed_ms = codex_result.as_ref().ok().map(|result| result.elapsed_ms);
        let mut closeout_events = Vec::new();
        let mut closeout_sequence = 1_usize;
        close_managed_git_worktree(
            manifest,
            &workspace,
            git_task_worktree_path
                .as_deref()
                .expect("managed Git mode prepared its closeout worktree"),
            lifecycle,
            model_request_state_result
                .as_ref()
                .is_ok_and(|state| state.terminal_state != "incomplete"),
            &mut closeout_events,
            &mut closeout_sequence,
        )?;
        write_json_lines_new(
            &run_dir.join("private/managed-worktree-closeout-events.jsonl"),
            &closeout_events,
        )?;
        write_json_new(&run_dir.join("managed-worktree-lifecycle.json"), lifecycle)?;
    }
    let codex = codex_result?;
    let model_request_state = model_request_state_result?;
    write_json_new(
        &run_dir.join("model-request-state.json"),
        &model_request_state,
    )?;
    let usage_result = import_executor_usage(
        manifest,
        &raw_events,
        &entry.run_id,
        &entry.workload_id,
        entry.mode,
    );
    let transcript_result = extract_and_validate_executor_transcript(
        manifest,
        &raw_events,
        &entry.run_id,
        entry.mode,
        git_start_state_proof.as_ref(),
    );
    let transcript = transcript_result.unwrap_or_else(|error| AgentTokenCommandTranscript {
        contract: crate::AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
        run_id: entry.run_id.clone(),
        mode: entry.mode,
        accounting_profile: manifest.accounting_profile,
        command_count: 0,
        commands: Vec::new(),
        valid: false,
        errors: vec![error],
        observed_required_commands: Vec::new(),
    });
    let mut secondary_metrics =
        extract_executor_secondary_metrics(manifest, &raw_events, &codex_stderr, &transcript)?;
    if let Some(lifecycle) = managed_worktree_lifecycle.as_ref() {
        secondary_metrics.host_worktree_provisioning_elapsed_ms =
            Some(lifecycle.provisioning_elapsed_ms);
        secondary_metrics.host_worktree_closeout_elapsed_ms = lifecycle.closeout_elapsed_ms;
    }
    write_command_events(&run_dir.join("command-events.jsonl"), &transcript)?;
    let usage_error = usage_result.as_ref().err().cloned();
    let provider_refusal = usage_result
        .as_ref()
        .ok()
        .is_some_and(|imported| imported.provider_refusal);
    let provider_stop_reason = usage_result
        .as_ref()
        .ok()
        .and_then(|imported| imported.provider_stop_reason.clone());
    let provider_completed_normally = match manifest.runtime.executor {
        crate::agent_token::AgentTokenExecutor::Codex => true,
        crate::agent_token::AgentTokenExecutor::Claude => {
            provider_stop_reason.as_deref() == Some("end_turn")
        }
    };
    let usage = usage_result.ok().map(|imported| imported.usage);
    let infrastructure_failure = classify_executor_infrastructure_failure(
        manifest,
        &raw_events,
        &codex_stderr,
        &codex,
        &transcript,
        usage.as_ref(),
    );
    if let Some(value) = &usage {
        write_json_line_new(&run_dir.join("provider-usage.jsonl"), value)?;
    } else {
        write_text_new(&run_dir.join("provider-usage.jsonl"), "")?;
    }

    let acceptance = run_acceptance(manifest, &receipt, &workspace, &run_dir)?;
    let browser = run_browser_acceptance(manifest, &receipt, &workspace, &run_dir)?;
    let workflow = verify_workflow(
        manifest,
        entry.mode,
        &entry.run_id,
        &workspace,
        git_worktree_path.as_deref(),
        git_start_state_proof.as_ref(),
        managed_worktree_lifecycle.as_ref(),
    )?;
    write_json_new(&run_dir.join("workflow-verification.json"), &workflow)?;

    let environment = AgentTokenEnvironment {
        contract: AGENT_TOKEN_ENVIRONMENT_CONTRACT.to_string(),
        captured_at: Utc::now().to_rfc3339(),
        os: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        codex_version: versions.codex.clone(),
        ait_version: versions.ait.clone(),
        git_version: versions.git.clone(),
        node_version: versions.node.clone(),
        browser_version: versions.browser.clone(),
        workflow_mode: workflow.workflow_mode.clone(),
        ait_edit_root_mode: manifest.ait_edit_root_mode,
        git_worktree_mode: manifest.git_worktree_mode,
        sprint_mode: workflow.sprint_mode.clone(),
        ait_server_connected: workflow.ait_server_configured,
        network_policy: manifest.network_policy.clone(),
        codex_permission_profile: permission_profile.name,
        codex_permission_profile_parent: permission_profile.extends,
        cache_class: manifest.cache_class.clone(),
        benchmark_enabled_feature_overrides: executor_enabled_feature_overrides(manifest),
        benchmark_disabled_feature_overrides: executor_disabled_feature_overrides(manifest),
        project_doc_max_bytes: manifest.runtime.project_doc_max_bytes,
    };
    write_json_new(&run_dir.join("environment.json"), &environment)?;

    let final_content_digest = digest_workspace(
        &workspace,
        &[
            ".ait".to_string(),
            ".git".to_string(),
            ".ait-worktree-links".to_string(),
            "AGENTS.md".to_string(),
        ],
    )
    .ok();
    let mut invalid_reasons = Vec::new();
    if let Some(error) = usage_error {
        invalid_reasons.push(format!(
            "provider usage or model-purity evidence is invalid: {error}"
        ));
    }
    if manifest.runtime.executor == crate::agent_token::AgentTokenExecutor::Claude
        && provider_stop_reason
            .as_deref()
            .is_some_and(|reason| reason != "end_turn" && reason != "refusal")
    {
        invalid_reasons.push(format!(
            "provider terminal stop reason {:?} is unsupported by the frozen Claude outcome contract",
            provider_stop_reason
        ));
    }
    if let Some(reason) = infrastructure_failure.as_deref() {
        invalid_reasons.push(format!("candidate infrastructure unavailable: {reason}"));
    }
    // A successful provider refusal is measured model behavior. It may stop
    // before issuing any repository command, so the missing workflow
    // transcript is a functional failure rather than invalid evidence.
    if !transcript.valid && !provider_refusal {
        invalid_reasons.extend(transcript.errors.iter().cloned());
    }
    if receipt.content_digest != run_manifest.fixture_content_digest {
        invalid_reasons.push("fixture digest linkage drifted".to_string());
    }
    if entry.mode == AgentTokenMode::AitLinearSingleSession {
        if workflow.workflow_mode != "solo_local" {
            invalid_reasons.push("AIT workflow mode is not solo_local".to_string());
        }
        if workflow.sprint_mode != manifest.ait_sprint_mode.as_str() {
            invalid_reasons.push(format!(
                "AIT sprint mode is not {}",
                manifest.ait_sprint_mode.as_str()
            ));
        }
        if manifest.ait_sprint_mode == AgentTokenAitSprintMode::On
            && !provider_refusal
            && workflow.sprint_item_closed != Some(true)
        {
            invalid_reasons.push("AIT sprint item was not automatically closed".to_string());
        }
        if workflow.default_remote_present
            || workflow.remote_count.unwrap_or_default() != 0
            || workflow.ait_server_configured
        {
            invalid_reasons.push(
                "AIT run configured a default remote, remote authority, or ait-server".to_string(),
            );
        }
    }

    let evaluator_accepted = acceptance
        .get("accepted")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let evaluator_score = acceptance.get("score").and_then(serde_json::Value::as_u64);
    let mut failure_reasons = Vec::new();
    if codex.timed_out {
        failure_reasons.push("candidate agent timed out".to_string());
    }
    if codex.exit_code != Some(0) {
        failure_reasons.push(format!("candidate agent exited with {:?}", codex.exit_code));
    }
    if !evaluator_accepted {
        failure_reasons.push("functional acceptance rejected the candidate".to_string());
    }
    if browser.status != "passed" {
        failure_reasons.push(format!("browser acceptance status is {}", browser.status));
    }
    if !workflow.closed {
        failure_reasons.push("repository workflow did not close cleanly".to_string());
    }
    if provider_refusal {
        failure_reasons.push("provider returned a refusal".to_string());
    } else if !provider_completed_normally {
        failure_reasons.push(format!(
            "provider terminal stop reason is {:?}, expected end_turn",
            provider_stop_reason
        ));
    }
    let valid_attempt = invalid_reasons.is_empty();
    let accepted_equivalent = valid_attempt
        && !codex.timed_out
        && codex.exit_code == Some(0)
        && evaluator_accepted
        && browser.status == "passed"
        && workflow.closed
        && provider_completed_normally;
    let summary = AgentTokenRunSummary {
        contract: AGENT_TOKEN_RUN_SUMMARY_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        run_id: entry.run_id.clone(),
        workload_id: entry.workload_id.clone(),
        mode: entry.mode,
        accounting_profile: manifest.accounting_profile,
        attempt: entry.attempt,
        block_index: entry.block_index,
        randomized_order: entry.randomized_order,
        initial_content_digest: receipt.content_digest,
        final_content_digest,
        codex_exit_code: codex.exit_code,
        codex_timed_out: codex.timed_out,
        elapsed_ms: codex.elapsed_ms,
        infrastructure_failure,
        provider_refusal,
        provider_stop_reason,
        usage,
        transcript,
        secondary_metrics,
        evaluator_exit_code: acceptance
            .get("_evaluator_exit_code")
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| i32::try_from(value).ok()),
        evaluator_score,
        evaluator_accepted,
        browser,
        workflow_closed: workflow.closed,
        valid_attempt,
        accepted_equivalent,
        invalid_reasons,
        failure_reasons,
    };
    write_json_new(&run_dir.join("run-summary.json"), &summary)?;
    Ok(summary)
}

fn sprint_plan_ref(run_id: &str) -> String {
    format!("agent-token-benchmark/{run_id}/root")
}

fn sprint_item_ref(run_id: &str) -> String {
    format!("agent-token-benchmark/{run_id}/implement")
}

fn sprint_card_template(run_id: &str) -> String {
    format!(
        "# Benchmark task [plan-ref: {plan_ref}]\n\n## Work item\n\n- [ ] Complete the shared benchmark task and its required validation. [ref: {item_ref}]\n",
        plan_ref = sprint_plan_ref(run_id),
        item_ref = sprint_item_ref(run_id),
    )
}

fn build_measured_prompt(
    manifest: &AgentTokenCampaignManifest,
    entry: &AgentTokenScheduleEntry,
    shared_task: &str,
    git_worktree_path: Option<&Path>,
    git_metadata_path: Option<&Path>,
    ait_edit_root: Option<&Path>,
) -> String {
    let profile = manifest.accounting_profile.as_str();
    let workflow = match (entry.mode, manifest.accounting_profile) {
        (
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        ) => {
            let worktree = git_worktree_path
                .expect("Git prompt requires its benchmark-owned linked-worktree path");
            if manifest.git_worktree_mode
                == AgentTokenGitWorktreeMode::CodexAppEquivalentManaged
            {
                format!(
                    "Use only the prepared local Git worktree and do not invoke `ait`. You already begin inside the detached managed worktree `{worktree}`, created by the benchmark host from its proven clean `main` HEAD before this model session. Perform every product edit and project validation in the current worktree. Leave the resulting changes in place for host closeout; do not create commits, branches, tags, stashes, or additional worktrees, and do not run Git add, checkout, switch, restore, reset, clean, merge, rebase, cherry-pick, revert, fetch, pull, push, remote, worktree add/remove, or any command that changes Git metadata, refs, index, or registration. Local read-only inspection of this repository is neither required nor prohibited. Do not leave the current worktree or access the parent main worktree. The host will record commit, fast-forward, and cleanup after the terminal model event; those host operations are outside model-token accounting.",
                    worktree = worktree.display(),
                )
            } else {
                format!(
                "Use only the prepared local Git repository through `{git}` and do not invoke `ait`. Begin in the clean `main` worktree. The runner has proven that the current `HEAD` is this clean `main`, so create exactly one linked worktree with either `{git} worktree add -b benchmark-task {worktree} main` or the equivalent `{git} worktree add -b benchmark-task {worktree}`; no other start point is allowed. Perform every product edit and project validation inside `{worktree}`. After validation, create exactly one candidate commit there. Return to the original `main` worktree, run `{git} merge --ff-only benchmark-task`, `{git} worktree remove {worktree}`, and `{git} branch -d benchmark-task`. These commands complete the measured local lifecycle. Leave `main` clean with the linked worktree and temporary branch removed. Do not copy or redirect `.git`, set `GIT_DIR` or `GIT_WORK_TREE`, or invoke clone, fetch, pull, push, remote, or `ls-remote`. Local read-only inspection of this repository is neither required nor prohibited.",
                git = manifest.runtime.git_program.display(),
                worktree = worktree.display(),
                )
            }
        }
        (
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::FirstUseTotalCost,
        ) => {
            let worktree = git_worktree_path
                .expect("Git prompt requires its benchmark-owned linked-worktree path");
            let metadata = git_metadata_path
                .expect("first-use Git prompt requires its writable metadata path");
            format!(
                "Use only the local Git repository through `{git}` and do not invoke `ait`. Run `{git} init --initial-branch=main --separate-git-dir {metadata} .`, set repository-local `user.name` to `AIT Benchmark Agent` and `user.email` to `benchmark-agent@example.invalid`, and create exactly one baseline commit before editing. Create exactly one linked worktree with `{git} worktree add -b benchmark-task {worktree} main` and perform every product edit and project validation inside `{worktree}`. After validation, create exactly one candidate commit there. Return to the original `main` worktree, run `{git} merge --ff-only benchmark-task`, `{git} worktree remove {worktree}`, and `{git} branch -d benchmark-task`. Leave `main` clean with the linked worktree and temporary branch removed. Do not copy or otherwise redirect `.git`, set `GIT_DIR` or `GIT_WORK_TREE`, or invoke clone, fetch, pull, push, remote, or `ls-remote`. Local read-only inspection of this repository is neither required nor prohibited.",
                git = manifest.runtime.git_program.display(),
                worktree = worktree.display(),
                metadata = metadata.display(),
            )
        }
        (
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        ) => {
            let edit_root = ait_edit_root.map(Path::display);
            match manifest.ait_sprint_mode {
            AgentTokenAitSprintMode::Off
                if manifest.ait_edit_root_mode
                    == crate::agent_token::AgentTokenAitEditRootMode::Returned =>
            {
                format!(
                "Use the prepared local AIT repository through `{ait}`. Start exactly one unbound task with `{ait} task start --title ... --intent ... --local --json`, retain the returned `task_id`, and enter the returned physical `edit_root` using `next_action.command`. Edit and run project validation there. Pass that `task_id` directly to `{ait} task finish <returned-task-id> --message ... --local --json`. `{ait} task start`, `{ait} task finish`, and `{ait} snapshot create --message ... --json` only when an intermediate checkpoint is necessary, are the complete AIT lifecycle command set for this run; do not invoke any additional AIT lifecycle or management command. Local read-only inspection of this repository is neither required nor prohibited. Do not invoke `git` for any purpose, including after project validation. This candidate intentionally has no Git repository.",
                ait = manifest.runtime.ait_program.display(),
                )
            }
            AgentTokenAitSprintMode::Off => format!(
                "Use the prepared local AIT repository through `{ait}`. Start exactly one unbound task and enter its worktree in one step with `{ait} task start --title ... --intent ... --edit-root {edit_root} --local --json && cd {edit_root}`. Retain the returned `task_id`. Edit and run project validation in `{edit_root}`. Pass that `task_id` directly to `{ait} task finish <returned-task-id> --message ... --local --json`. `{ait} task start`, `{ait} task finish`, and `{ait} snapshot create --message ... --json` only when an intermediate checkpoint is necessary, are the complete AIT lifecycle command set for this run; do not invoke any additional AIT lifecycle or management command. Local read-only inspection of this repository is neither required nor prohibited. Do not invoke `git` for any purpose, including after project validation. This candidate intentionally has no Git repository.",
                ait = manifest.runtime.ait_program.display(),
                edit_root = edit_root
                    .expect("explicit AIT prompt requires its benchmark-owned edit root"),
            ),
            AgentTokenAitSprintMode::On => format!(
                "Use the prepared local AIT repository through `{ait}`. Sprint mode is on. Before starting code work, author exactly `{card_path}` with the following Markdown (including the exact refs):\n\n```markdown\n{card}```\n\nStart exactly one bound task and enter its worktree in one step with `{ait} task start --from {card_path}#{item_ref} --intent ... --edit-root {edit_root} --local --json && cd {edit_root}`; do not run a separate plan sync. Retain the returned `task_id`. Edit and run project validation in `{edit_root}`. Pass that `task_id` directly to `{ait} task finish <returned-task-id> --message ... --local --json`; successful finish must automatically close the exact sprint checklist item. `{ait} task start`, `{ait} task finish`, and `{ait} snapshot create --message ... --json` only when an intermediate checkpoint is necessary, are the complete AIT lifecycle command set for this run; do not invoke any additional AIT lifecycle or management command. Local read-only inspection of this repository is neither required nor prohibited. Do not invoke `git` for any purpose, including after project validation. This candidate intentionally has no Git repository.",
                ait = manifest.runtime.ait_program.display(),
                card_path = AIT_SPRINT_CARD_PATH,
                card = sprint_card_template(&entry.run_id),
                item_ref = sprint_item_ref(&entry.run_id),
                edit_root = edit_root
                    .expect("sprint-on AIT prompt requires its benchmark-owned edit root"),
            ),
        }
        },
        (
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::FirstUseTotalCost,
        ) => format!(
            "Use the local AIT repository through `{ait}`. Run `{ait} init`, then `{ait} config set --workflow-mode solo_local --sprint off --default-author-mode ai_only_experimental --default-model {model} --user-name benchmark-agent --user-email benchmark-agent@example.invalid --json`, and create the baseline with `{ait} snapshot create --message ... --json`. Start exactly one unbound task with `{ait} task start --title ... --intent ... --local --json`, retain the returned `task_id`, and enter the returned physical `edit_root` using `next_action.command`. Edit and run project validation there, then pass that `task_id` directly to `{ait} task finish <returned-task-id> --message ... --local --json`. Local read-only inspection of this repository is neither required nor prohibited. Do not invoke `git` for any purpose, including after project validation. This candidate intentionally has no Git repository.",
            ait = manifest.runtime.ait_program.display(),
            model = manifest.model.model_id,
        ),
    };
    format!(
        "You are one fresh measured coding session in the AIT agent-token game-development benchmark. Complete the shared task below without asking for help or receiving human repair.\n\nFairness rules:\n- Work only inside this fresh candidate repository and its declared local task worktree.\n- Do not access the public network, install dependencies, use hosted assets, or inspect benchmark evaluator source outside the candidate repository.\n- Do not add Python. Use project-local Node validation and preserve unrelated behavior.\n- The accounting profile is `{profile}` and the workload is `{workload}`.\n- Run the project-local validation before repository closeout.\n\nRepository workflow:\n{workflow}\n\nShared task (identical outcome contract in both modes):\n\n{shared_task}",
        workload = entry.workload_id,
    )
}

fn bootstrap_git(
    manifest: &AgentTokenCampaignManifest,
    workspace: &Path,
    metadata_dir: &Path,
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<(), String> {
    run_checked_event(
        &manifest.runtime.git_program,
        &[
            "init",
            "--initial-branch=main",
            "--separate-git-dir",
            metadata_dir.to_str().ok_or_else(|| {
                format!(
                    "Git metadata path is not valid UTF-8: {}",
                    metadata_dir.display()
                )
            })?,
            ".",
        ],
        workspace,
        "bootstrap",
        events,
        sequence,
    )?;
    for args in [
        vec!["config", "user.name", "AIT Benchmark Agent"],
        vec!["config", "user.email", "benchmark-agent@example.invalid"],
        vec!["add", "--all"],
        vec!["commit", "-m", "Benchmark fixture baseline"],
    ] {
        run_checked_event(
            &manifest.runtime.git_program,
            &args,
            workspace,
            "bootstrap",
            events,
            sequence,
        )?;
    }
    Ok(())
}

fn provision_managed_git_worktree(
    manifest: &AgentTokenCampaignManifest,
    entry: &AgentTokenScheduleEntry,
    workspace: &Path,
    worktree: &Path,
    starting_head_oid: &str,
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<AgentTokenManagedWorktreeLifecycle, String> {
    let provision_started_at = Utc::now().to_rfc3339();
    let start = Instant::now();
    let worktree_text = worktree.to_str().ok_or_else(|| {
        format!(
            "Managed Git worktree path is not valid UTF-8: {}",
            worktree.display()
        )
    })?;
    run_checked_event(
        &manifest.runtime.git_program,
        &["worktree", "add", "--detach", worktree_text, "HEAD"],
        workspace,
        "host-managed-worktree-provision",
        events,
        sequence,
    )?;
    let ready_head = command_output(
        &manifest.runtime.git_program,
        &["rev-parse", "--verify", "HEAD"],
        worktree,
    )?
    .trim()
    .to_string();
    if ready_head != starting_head_oid {
        return Err(format!(
            "Managed worktree ready HEAD {ready_head} differs from proven start {starting_head_oid}"
        ));
    }
    Ok(AgentTokenManagedWorktreeLifecycle {
        contract: AGENT_TOKEN_MANAGED_WORKTREE_LIFECYCLE_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        run_id: entry.run_id.clone(),
        boundary: AgentTokenGitWorktreeMode::CodexAppEquivalentManaged
            .as_str()
            .to_string(),
        desktop_private_ipc_invoked: false,
        worktree_path: worktree.display().to_string(),
        provision_started_at,
        ready_at: Utc::now().to_rfc3339(),
        provisioning_elapsed_ms: elapsed_millis(start),
        model_started_at: None,
        model_completed_at: None,
        model_elapsed_ms: None,
        closeout_started_at: None,
        closed_at: None,
        closeout_elapsed_ms: None,
        starting_head_oid: starting_head_oid.to_string(),
        worktree_ready_head_oid: Some(ready_head),
        candidate_head_oid: None,
        final_main_head_oid: None,
        worktree_created_before_model: true,
        host_closeout_after_terminal_model_event: false,
        host_commit_created: false,
        main_fast_forwarded: false,
        worktree_removed: false,
        closed: false,
        failure_reasons: Vec::new(),
    })
}

fn close_managed_git_worktree(
    manifest: &AgentTokenCampaignManifest,
    workspace: &Path,
    worktree: &Path,
    lifecycle: &mut AgentTokenManagedWorktreeLifecycle,
    terminal_model_event_observed: bool,
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<(), String> {
    lifecycle.closeout_started_at = Some(Utc::now().to_rfc3339());
    lifecycle.host_closeout_after_terminal_model_event = terminal_model_event_observed;
    if !terminal_model_event_observed {
        lifecycle
            .failure_reasons
            .push("Host closeout began without a terminal model event".to_string());
    }
    let start = Instant::now();
    let worktree_text = worktree.to_str().ok_or_else(|| {
        format!(
            "Managed Git worktree path is not valid UTF-8: {}",
            worktree.display()
        )
    })?;
    let observed_head = command_output(
        &manifest.runtime.git_program,
        &["rev-parse", "--verify", "HEAD"],
        worktree,
    );
    if observed_head
        .as_ref()
        .is_ok_and(|head| head.trim() != lifecycle.starting_head_oid)
    {
        lifecycle
            .failure_reasons
            .push("Measured session moved detached HEAD before host closeout".to_string());
    }
    if let Err(error) = observed_head.as_ref() {
        lifecycle
            .failure_reasons
            .push(format!("Failed to inspect managed-worktree HEAD: {error}"));
    }

    let add_ok = run_recorded_event(
        &manifest.runtime.git_program,
        &["add", "--all"],
        worktree,
        "host-managed-worktree-closeout",
        events,
        sequence,
    )?;
    if !add_ok {
        lifecycle
            .failure_reasons
            .push("Host could not stage managed-worktree changes".to_string());
    }
    let commit_ok = add_ok
        && run_recorded_event(
            &manifest.runtime.git_program,
            &[
                "commit",
                "--allow-empty",
                "-m",
                "Benchmark host-managed candidate",
            ],
            worktree,
            "host-managed-worktree-closeout",
            events,
            sequence,
        )?;
    lifecycle.host_commit_created = commit_ok;
    if !commit_ok {
        lifecycle
            .failure_reasons
            .push("Host could not create the managed candidate commit".to_string());
    }

    if commit_ok {
        match command_output(
            &manifest.runtime.git_program,
            &["rev-parse", "--verify", "HEAD"],
            worktree,
        ) {
            Ok(head) => lifecycle.candidate_head_oid = Some(head.trim().to_string()),
            Err(error) => lifecycle
                .failure_reasons
                .push(format!("Failed to resolve managed candidate HEAD: {error}")),
        }
    }
    if let Some(candidate) = lifecycle.candidate_head_oid.clone() {
        lifecycle.main_fast_forwarded = run_recorded_event(
            &manifest.runtime.git_program,
            &["merge", "--ff-only", candidate.as_str()],
            workspace,
            "host-managed-worktree-closeout",
            events,
            sequence,
        )?;
        if !lifecycle.main_fast_forwarded {
            lifecycle
                .failure_reasons
                .push("Host could not fast-forward main to the managed candidate".to_string());
        }
    }

    lifecycle.worktree_removed = run_recorded_event(
        &manifest.runtime.git_program,
        &["worktree", "remove", worktree_text],
        workspace,
        "host-managed-worktree-closeout",
        events,
        sequence,
    )?;
    if !lifecycle.worktree_removed {
        lifecycle
            .failure_reasons
            .push("Host could not remove the managed worktree cleanly".to_string());
        let _ = run_recorded_event(
            &manifest.runtime.git_program,
            &["worktree", "remove", "--force", worktree_text],
            workspace,
            "host-managed-worktree-cleanup",
            events,
            sequence,
        )?;
    }
    if let Ok(head) = command_output(
        &manifest.runtime.git_program,
        &["rev-parse", "--verify", "HEAD"],
        workspace,
    ) {
        lifecycle.final_main_head_oid = Some(head.trim().to_string());
    }
    if lifecycle.final_main_head_oid != lifecycle.candidate_head_oid {
        lifecycle
            .failure_reasons
            .push("Final main HEAD differs from the managed candidate".to_string());
    }
    if worktree.exists() {
        lifecycle
            .failure_reasons
            .push("Managed worktree path still exists after host closeout".to_string());
    }
    lifecycle.closeout_elapsed_ms = Some(elapsed_millis(start));
    lifecycle.closed_at = Some(Utc::now().to_rfc3339());
    lifecycle.closed = lifecycle.failure_reasons.is_empty()
        && lifecycle.host_commit_created
        && lifecycle.main_fast_forwarded
        && lifecycle.worktree_removed;
    Ok(())
}

fn capture_git_start_state_proof(
    manifest: &AgentTokenCampaignManifest,
    run_id: &str,
    workspace: &Path,
) -> AgentTokenGitStartStateProof {
    let branch = command_output(
        &manifest.runtime.git_program,
        &["symbolic-ref", "--short", "HEAD"],
        workspace,
    );
    let head = command_output(
        &manifest.runtime.git_program,
        &["rev-parse", "--verify", "HEAD"],
        workspace,
    );
    let main = command_output(
        &manifest.runtime.git_program,
        &["rev-parse", "--verify", "refs/heads/main"],
        workspace,
    );
    let status = command_output(
        &manifest.runtime.git_program,
        &["status", "--porcelain"],
        workspace,
    );

    let current_branch = branch.as_ref().ok().map(|value| value.trim().to_string());
    let head_oid = head.as_ref().ok().map(|value| value.trim().to_string());
    let main_oid = main.as_ref().ok().map(|value| value.trim().to_string());
    let status_porcelain = status.as_ref().ok().map(ToString::to_string);
    let clean = status.as_ref().is_ok_and(|output| output.trim().is_empty());
    let head_matches_main = head_oid
        .as_ref()
        .zip(main_oid.as_ref())
        .is_some_and(|(head, main)| !head.is_empty() && head == main);
    let mut failure_reasons = Vec::new();
    for (field, result) in [
        ("current branch", &branch),
        ("HEAD", &head),
        ("refs/heads/main", &main),
        ("porcelain status", &status),
    ] {
        if let Err(error) = result {
            failure_reasons.push(format!("Failed to prove Git {field}: {error}"));
        }
    }
    if current_branch.as_deref() != Some("main") {
        failure_reasons.push(format!(
            "Git provider turn would start on {:?} instead of main",
            current_branch.as_deref()
        ));
    }
    if !head_matches_main {
        failure_reasons.push("Git HEAD does not equal refs/heads/main".to_string());
    }
    if !clean {
        failure_reasons.push("Git main worktree is not clean before the provider turn".to_string());
    }
    AgentTokenGitStartStateProof {
        contract: AGENT_TOKEN_GIT_START_STATE_PROOF_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        run_id: run_id.to_string(),
        captured_at: Utc::now().to_rfc3339(),
        current_branch,
        head_oid,
        main_oid,
        status_porcelain,
        clean,
        head_matches_main,
        passed: failure_reasons.is_empty(),
        failure_reasons,
    }
}

fn bootstrap_ait(
    manifest: &AgentTokenCampaignManifest,
    workspace: &Path,
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<AitBootstrap, String> {
    run_checked_event(
        &manifest.runtime.ait_program,
        &["init", "--json"],
        workspace,
        "bootstrap",
        events,
        sequence,
    )?;
    let model = manifest.model.model_id.as_str();
    let sprint_mode = manifest.ait_sprint_mode.as_str();
    run_checked_event(
        &manifest.runtime.ait_program,
        &[
            "config",
            "set",
            "--workflow-mode",
            "solo_local",
            "--sprint",
            sprint_mode,
            "--task-review",
            "automatic",
            "--default-author-mode",
            "ai_only_experimental",
            "--default-model",
            model,
            "--user-name",
            "benchmark-agent",
            "--user-email",
            "benchmark-agent@example.invalid",
            "--json",
        ],
        workspace,
        "bootstrap",
        events,
        sequence,
    )?;
    // `ait init` and `ait config set` generate an AGENTS.md and a docs/ tree.
    // No measured Git workspace lists either, and across eleven GD-02
    // diagnostic lanes agents spent 1.6-3.0 model requests per lane exploring
    // exactly these artifacts, a structural tax charged only to AIT. Delete
    // AGENTS.md and archive its Plan through a runner-owned prune sync; the
    // full lifecycle was verified working without it. The .40 stub approach is
    // retired: it removed the guidance bytes but left the exploration bait.
    // Sprint-on keeps docs/ because authoring the card inside it is the
    // measured treatment; sprint-off removes it too.
    let project_document_path = workspace.join(crate::agent_token::AIT_PURGED_PROJECT_DOCUMENT);
    if project_document_path.exists() {
        // Mirror the generated guidance into the executor'"'"'s native auto-load
        // channel before removing the file Claude never auto-loads. Marker
        // tests: Claude Code auto-loads CLAUDE.md under the project setting
        // source and never auto-loads AGENTS.md, so this delivers the guidance
        // at zero exploration requests.
        let guidance = fs::read_to_string(&project_document_path).map_err(|error| {
            format!(
                "Failed to read the generated project document {}: {error}",
                project_document_path.display()
            )
        })?;
        fs::write(workspace.join("CLAUDE.md"), guidance)
            .map_err(|error| format!("Failed to mirror guidance into CLAUDE.md: {error}"))?;
        fs::remove_file(&project_document_path).map_err(|error| {
            format!(
                "Failed to remove the generated project document {}: {error}",
                project_document_path.display()
            )
        })?;
        run_checked_event(
            &manifest.runtime.ait_program,
            &[
                "plan",
                "sync",
                crate::agent_token::AIT_PURGED_PROJECT_DOCUMENT,
                "--prune",
                "--local",
            ],
            workspace,
            "bootstrap",
            events,
            sequence,
        )?;
    }
    if manifest.ait_sprint_mode == AgentTokenAitSprintMode::Off {
        let docs = workspace.join("docs");
        if docs.exists() {
            fs::remove_dir_all(&docs).map_err(|error| {
                format!(
                    "Failed to remove the generated docs tree {}: {error}",
                    docs.display()
                )
            })?;
        }
    }
    if project_document_path.exists() {
        return Err("Measured AIT workspace still lists AGENTS.md after the purge".to_string());
    }
    run_checked_event(
        &manifest.runtime.ait_program,
        &[
            "snapshot",
            "create",
            "--message",
            "Benchmark fixture baseline",
            "--json",
        ],
        workspace,
        "bootstrap",
        events,
        sequence,
    )?;
    let config = command_json(
        &manifest.runtime.ait_program,
        &["config", "show", "--json"],
        workspace,
    )?;
    validate_solo_local_config(&config, manifest.ait_sprint_mode)?;
    let worktree_add_dir = config
        .pointer("/task_worktree/ephemeral_root/value")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| {
            "AIT config did not expose task_worktree.ephemeral_root.value".to_string()
        })?;
    Ok(AitBootstrap { worktree_add_dir })
}

fn permission_profile_path(path: &Path, kind: &str) -> Result<String, String> {
    if !path.is_absolute() {
        return Err(format!(
            "Codex permission profile {kind} path must be absolute: {}",
            path.display()
        ));
    }
    path.to_str().map(str::to_string).ok_or_else(|| {
        format!(
            "Codex permission profile {kind} path is not valid UTF-8: {}",
            path.display()
        )
    })
}

fn build_codex_permission_profile(
    primary_workspace: &Path,
    additional_workspace_roots: &[PathBuf],
    git_write_exceptions: &[PathBuf],
) -> Result<AgentTokenCodexPermissionProfile, String> {
    let primary_workspace_text = permission_profile_path(primary_workspace, "primary workspace")?;
    let mut roots = BTreeSet::new();
    for root in additional_workspace_roots {
        roots.insert(permission_profile_path(root, "additional workspace root")?);
    }
    let root_paths = roots.iter().map(PathBuf::from).collect::<Vec<_>>();
    let mut exceptions = BTreeSet::new();
    for exception in git_write_exceptions {
        let in_declared_root = exception.starts_with(primary_workspace)
            || root_paths.iter().any(|root| exception.starts_with(root));
        if !in_declared_root {
            return Err(format!(
                "Codex Git write exception is outside the declared workspaces: {}",
                exception.display()
            ));
        }
        exceptions.insert(permission_profile_path(exception, "Git write exception")?);
    }
    Ok(AgentTokenCodexPermissionProfile {
        contract: AGENT_TOKEN_CODEX_PERMISSION_PROFILE_CONTRACT.to_string(),
        name: CODEX_PERMISSION_PROFILE_NAME.to_string(),
        extends: CODEX_PERMISSION_PROFILE_PARENT.to_string(),
        network_enabled: false,
        primary_workspace: primary_workspace_text,
        additional_workspace_roots: roots.into_iter().collect(),
        git_write_exceptions: exceptions.into_iter().collect(),
    })
}

fn toml_basic_string(value: &str) -> Result<String, String> {
    serde_json::to_string(value)
        .map_err(|error| format!("Failed to encode Codex permission-profile string: {error}"))
}

fn codex_permission_profile_override(
    profile: &AgentTokenCodexPermissionProfile,
) -> Result<String, String> {
    let workspace_roots = profile
        .additional_workspace_roots
        .iter()
        .map(|path| Ok(format!("{}=true", toml_basic_string(path)?)))
        .collect::<Result<Vec<_>, String>>()?
        .join(", ");
    let git_write_exceptions = profile
        .git_write_exceptions
        .iter()
        .map(|path| Ok(format!("{}=\"write\"", toml_basic_string(path)?)))
        .collect::<Result<Vec<_>, String>>()?
        .join(", ");
    Ok(format!(
        "permissions.{}={{ description=\"AIT benchmark local no-network profile\", extends={}, workspace_roots={{ {} }}, filesystem={{ {} }}, network={{ enabled=false }} }}",
        profile.name,
        toml_basic_string(&profile.extends)?,
        workspace_roots,
        git_write_exceptions,
    ))
}

fn codex_default_permission_override() -> Result<String, String> {
    Ok(format!(
        "default_permissions={}",
        toml_basic_string(CODEX_PERMISSION_PROFILE_NAME)?
    ))
}

fn build_codex_sandbox_command(
    manifest: &AgentTokenCampaignManifest,
    profile: &AgentTokenCodexPermissionProfile,
    cwd: &Path,
    program: &Path,
    args: &[String],
) -> Result<Command, String> {
    let permission_profile = codex_permission_profile_override(profile)?;
    let mut command = Command::new(&manifest.runtime.codex_program);
    command
        .arg("sandbox")
        .arg("--permission-profile")
        .arg(&profile.name)
        .arg("--config")
        .arg(permission_profile)
        .arg("--cd")
        .arg(cwd)
        .arg("--")
        .arg(program)
        .args(args)
        .env("NO_COLOR", "1");
    Ok(command)
}

fn run_codex_sandbox_event(
    manifest: &AgentTokenCampaignManifest,
    profile: &AgentTokenCodexPermissionProfile,
    cwd: &Path,
    program: &Path,
    args: &[String],
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<bool, String> {
    let mut command = build_codex_sandbox_command(manifest, profile, cwd, program, args)?;
    let recorded_args = command
        .get_args()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let output = command.output().map_err(|error| {
        format!(
            "Failed to launch Codex Git-worktree permission probe {}: {error}",
            manifest.runtime.codex_program.display()
        )
    })?;
    let success = output.status.success();
    events.push(ExternalCommandEvent {
        sequence: *sequence,
        phase: "codex-permission-profile-probe".to_string(),
        program: manifest.runtime.codex_program.display().to_string(),
        args: recorded_args,
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    });
    *sequence += 1;
    Ok(success)
}

fn run_git_worktree_permission_preflight(
    manifest: &AgentTokenCampaignManifest,
    campaign_dir: &Path,
    versions: &CapturedVersions,
) -> Result<AgentTokenGitWorktreePermissionPreflightReport, String> {
    const REQUIRED_COMMAND_COUNT: usize = 5;
    const PROBE_BRANCH: &str = "benchmark-permission-probe";
    let runtime_root = campaign_dir.join("private/git-worktree-permission-preflight-runtime");
    prepare_empty_directory(&runtime_root, "Git worktree permission-preflight runtime")?;
    let workspace = runtime_root.join("workspace");
    let metadata = runtime_root.join("git-metadata");
    let worktree_container = runtime_root.join("worktree-container");
    let task_worktree = worktree_container.join("task");
    prepare_empty_directory(&workspace, "Git permission-preflight workspace")?;
    prepare_empty_directory(&metadata, "Git permission-preflight metadata")?;
    prepare_empty_directory(
        &worktree_container,
        "Git permission-preflight worktree container",
    )?;
    write_text_new(
        &workspace.join("permission-probe.txt"),
        "permission probe\n",
    )?;

    let mut events = Vec::new();
    let mut sequence = 1_usize;
    bootstrap_git(manifest, &workspace, &metadata, &mut events, &mut sequence)?;
    let profile = build_codex_permission_profile(
        &workspace,
        &[metadata.clone(), worktree_container],
        &[
            workspace.join(".git"),
            metadata.clone(),
            task_worktree.join(".git"),
        ],
    )?;
    write_json_new(
        &campaign_dir.join("git-worktree-permission-profile.json"),
        &profile,
    )?;

    let commands = [
        (
            workspace.as_path(),
            vec![
                "worktree".to_string(),
                "add".to_string(),
                "-b".to_string(),
                PROBE_BRANCH.to_string(),
                permission_profile_path(&task_worktree, "permission-preflight task worktree")?,
                "main".to_string(),
            ],
        ),
        (
            task_worktree.as_path(),
            vec![
                "commit".to_string(),
                "--allow-empty".to_string(),
                "-m".to_string(),
                "Codex permission-profile probe".to_string(),
            ],
        ),
        (
            workspace.as_path(),
            vec![
                "merge".to_string(),
                "--ff-only".to_string(),
                PROBE_BRANCH.to_string(),
            ],
        ),
        (
            workspace.as_path(),
            vec![
                "worktree".to_string(),
                "remove".to_string(),
                permission_profile_path(&task_worktree, "permission-preflight task worktree")?,
            ],
        ),
        (
            workspace.as_path(),
            vec![
                "branch".to_string(),
                "-d".to_string(),
                PROBE_BRANCH.to_string(),
            ],
        ),
    ];
    let mut successful_command_count = 0_usize;
    for (cwd, args) in &commands {
        if !run_codex_sandbox_event(
            manifest,
            &profile,
            cwd,
            &manifest.runtime.git_program,
            args,
            &mut events,
            &mut sequence,
        )? {
            break;
        }
        successful_command_count += 1;
    }
    write_json_lines_new(
        &campaign_dir.join("private/git-worktree-permission-preflight-events.jsonl"),
        &events,
    )?;

    let mut failure_reasons = Vec::new();
    let executed_command_count = events
        .iter()
        .filter(|event| event.phase == "codex-permission-profile-probe")
        .count();
    if executed_command_count != REQUIRED_COMMAND_COUNT
        || successful_command_count != REQUIRED_COMMAND_COUNT
    {
        failure_reasons.push(format!(
            "Codex permission profile completed {successful_command_count}/{REQUIRED_COMMAND_COUNT} Git lifecycle commands"
        ));
    }
    let status = command_output(
        &manifest.runtime.git_program,
        &["status", "--porcelain"],
        &workspace,
    );
    let main_clean = status.as_ref().is_ok_and(|output| output.trim().is_empty());
    if !main_clean {
        failure_reasons.push(match status {
            Ok(_) => "Git permission preflight left main dirty".to_string(),
            Err(error) => error,
        });
    }
    let worktrees = command_output(
        &manifest.runtime.git_program,
        &["worktree", "list", "--porcelain"],
        &workspace,
    );
    let registered_worktree_count = worktrees.as_ref().ok().map(|output| {
        output
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count()
    });
    if registered_worktree_count != Some(1) {
        failure_reasons.push(format!(
            "Git permission preflight retained {:?} registered worktrees; expected one",
            registered_worktree_count
        ));
    }
    let branch = command_output(
        &manifest.runtime.git_program,
        &["branch", "--list", PROBE_BRANCH],
        &workspace,
    );
    let temporary_branch_absent = branch.as_ref().is_ok_and(|output| output.trim().is_empty());
    if !temporary_branch_absent {
        failure_reasons.push(match branch {
            Ok(_) => "Git permission preflight retained its temporary branch".to_string(),
            Err(error) => error,
        });
    }
    let commit_count = command_output(
        &manifest.runtime.git_program,
        &["rev-list", "--count", "HEAD"],
        &workspace,
    );
    let main_commit_count = commit_count
        .as_ref()
        .ok()
        .and_then(|output| output.trim().parse::<u64>().ok());
    if main_commit_count != Some(2) {
        failure_reasons.push(format!(
            "Git permission preflight main has {:?} commits; expected baseline plus probe",
            main_commit_count
        ));
    }
    let task_path_absent = !task_worktree.exists();
    if !task_path_absent {
        failure_reasons.push(format!(
            "Git permission preflight did not delete {}",
            task_worktree.display()
        ));
    }
    let report = AgentTokenGitWorktreePermissionPreflightReport {
        contract: AGENT_TOKEN_GIT_WORKTREE_PERMISSION_PREFLIGHT_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        codex_version: versions.codex.clone(),
        git_version: versions.git.clone(),
        permission_profile: profile,
        required_command_count: REQUIRED_COMMAND_COUNT,
        executed_command_count,
        successful_command_count,
        main_clean,
        registered_worktree_count,
        temporary_branch_absent,
        main_commit_count,
        task_path_absent,
        passed: failure_reasons.is_empty(),
        failure_reasons,
    };
    write_json_new(
        &campaign_dir.join("git-worktree-permission-preflight-report.json"),
        &report,
    )?;
    Ok(report)
}

fn build_codex_command(
    manifest: &AgentTokenCampaignManifest,
    workspace: &Path,
    add_dirs: &[PathBuf],
    git_write_exceptions: &[PathBuf],
) -> Result<Command, String> {
    let project_doc_config = format!(
        "project_doc_max_bytes={}",
        manifest.runtime.project_doc_max_bytes
    );
    let profile = build_codex_permission_profile(workspace, add_dirs, git_write_exceptions)?;
    let default_permission = codex_default_permission_override()?;
    let permission_profile = codex_permission_profile_override(&profile)?;
    let mut command = Command::new(&manifest.runtime.codex_program);
    command.arg("exec");
    command
        .args([
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--strict-config",
            "--json",
            "--skip-git-repo-check",
            "--model",
            manifest.model.model_id.as_str(),
            "--config",
            &format!(
                "model_reasoning_effort=\"{}\"",
                manifest.model.reasoning_effort
            ),
            "--config",
            default_permission.as_str(),
            "--config",
            permission_profile.as_str(),
            "--config",
            project_doc_config.as_str(),
            "--cd",
        ])
        .arg(workspace);
    command.arg("-").env("NO_COLOR", "1");
    Ok(command)
}

fn run_codex(
    manifest: &AgentTokenCampaignManifest,
    workspace: &Path,
    add_dirs: &[PathBuf],
    git_write_exceptions: &[PathBuf],
    prompt: &str,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<TimedProcessResult, String> {
    let command = build_codex_command(manifest, workspace, add_dirs, git_write_exceptions)?;
    run_agent_process(
        command,
        "Codex",
        &manifest.runtime.codex_program,
        prompt,
        stdout_path,
        stderr_path,
        manifest.runtime.run_timeout_seconds,
    )
}

/// Launch a measured agent subject with the shared stdin-prompt, redirected
/// stream, process-group, and timeout contract used by every executor.
#[allow(clippy::too_many_arguments)]
fn run_agent_process(
    mut command: Command,
    executor_label: &str,
    program: &Path,
    prompt: &str,
    stdout_path: &Path,
    stderr_path: &Path,
    run_timeout_seconds: u64,
) -> Result<TimedProcessResult, String> {
    ensure_parent(stdout_path)?;
    ensure_parent(stderr_path)?;
    let stdout = create_new_file(stdout_path)?;
    let stderr = create_new_file(stderr_path)?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let start = Instant::now();
    let mut child = command.spawn().map_err(|error| {
        format!(
            "Failed to launch {executor_label} benchmark subject {}: {error}",
            program.display()
        )
    })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("{executor_label} benchmark subject has no stdin pipe"))?;
    stdin
        .write_all(prompt.as_bytes())
        .map_err(|error| format!("Failed to write {executor_label} benchmark prompt: {error}"))?;
    drop(stdin);
    wait_for_child(&mut child, Duration::from_secs(run_timeout_seconds), start)
}

/// Build the Claude Code headless invocation for one measured lane. The
/// command mirrors the codex executor boundary: workspace-scoped writes with
/// explicit additional roots, stream-json events on stdout, the pinned model
/// and reasoning effort, no MCP servers, and no session persistence. The
/// fixture workspace carries no CLAUDE.md, keeping project-document loading
/// symmetrically disabled across executors.
fn build_claude_command(
    manifest: &AgentTokenCampaignManifest,
    workspace: &Path,
    add_dirs: &[PathBuf],
    git_write_exceptions: &[PathBuf],
) -> Result<Command, String> {
    let program =
        manifest.runtime.claude_program.as_ref().ok_or_else(|| {
            "runtime.claude_program is required for the claude executor".to_string()
        })?;
    let mut command = Command::new(program);
    command.args([
        "--print",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
        "--strict-mcp-config",
        "--setting-sources",
        "project",
        "--prompt-suggestions",
        "false",
        "--settings",
        claude_sandbox_settings_json(add_dirs, git_write_exceptions)?.as_str(),
        "--model",
        manifest.model.model_id.as_str(),
        "--effort",
        manifest.model.reasoning_effort.as_str(),
    ]);
    command.arg("--tools");
    command.arg(CLAUDE_ALLOWED_TOOLS.join(","));
    command.arg("--allowed-tools");
    for tool in CLAUDE_ALLOWED_TOOLS {
        command.arg(tool);
    }
    command.arg("--disallowed-tools");
    for tool in CLAUDE_DISALLOWED_TOOLS {
        command.arg(tool);
    }
    // --add-dir accepts only existing directories: passing a gitfile write
    // exception or a not-yet-created path poisons the sandbox writable-path
    // profile and the sandbox then denies the declared worktree container
    // (probe-bisected). Every declared root and exception still reaches the
    // sandbox through filesystem.allowWrite above, which handles files and
    // not-yet-existing paths correctly.
    for dir in add_dirs.iter().chain(git_write_exceptions.iter()) {
        if dir.is_dir() {
            command.arg("--add-dir").arg(dir);
        }
    }
    command.current_dir(workspace);
    command.env("NO_COLOR", "1");
    command.env(
        crate::agent_token::CLAUDE_SINGLE_MODEL_ENV.0,
        crate::agent_token::CLAUDE_SINGLE_MODEL_ENV.1,
    );
    Ok(command)
}

fn run_claude(
    manifest: &AgentTokenCampaignManifest,
    workspace: &Path,
    add_dirs: &[PathBuf],
    git_write_exceptions: &[PathBuf],
    prompt: &str,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<TimedProcessResult, String> {
    let command = build_claude_command(manifest, workspace, add_dirs, git_write_exceptions)?;
    let program =
        manifest.runtime.claude_program.clone().ok_or_else(|| {
            "runtime.claude_program is required for the claude executor".to_string()
        })?;
    run_agent_process(
        command,
        "Claude",
        &program,
        prompt,
        stdout_path,
        stderr_path,
        manifest.runtime.run_timeout_seconds,
    )
}

/// Executor dispatch for one measured lane: run the configured agent, then
/// import usage and validate the command transcript through the matching
/// adapter. Every downstream consumer stays executor-agnostic.
fn run_measured_agent(
    manifest: &AgentTokenCampaignManifest,
    workspace: &Path,
    add_dirs: &[PathBuf],
    git_write_exceptions: &[PathBuf],
    prompt: &str,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<TimedProcessResult, String> {
    match manifest.runtime.executor {
        crate::agent_token::AgentTokenExecutor::Codex => run_codex(
            manifest,
            workspace,
            add_dirs,
            git_write_exceptions,
            prompt,
            stdout_path,
            stderr_path,
        ),
        crate::agent_token::AgentTokenExecutor::Claude => run_claude(
            manifest,
            workspace,
            add_dirs,
            git_write_exceptions,
            prompt,
            stdout_path,
            stderr_path,
        ),
    }
}

fn inspect_model_request_state(
    manifest: &AgentTokenCampaignManifest,
    source: &Path,
    run_id: &str,
) -> Result<AgentTokenModelRequestState, String> {
    let source_text = fs::read_to_string(source).map_err(|error| {
        format!(
            "Failed to read model request state source {}: {error}",
            source.display()
        )
    })?;
    let mut state = AgentTokenModelRequestState {
        contract: AGENT_TOKEN_MODEL_REQUEST_STATE_CONTRACT.to_string(),
        run_id: run_id.to_string(),
        executor: manifest.runtime.executor.as_str().to_string(),
        thread_id: None,
        turn_started_count: 0,
        turn_completed_count: 0,
        turn_failed_count: 0,
        usage_event_count: 0,
        model_reroute_event_count: 0,
        safety_event_count: 0,
        provider_error_event_count: 0,
        terminal_state: "incomplete".to_string(),
        internal_provider_request_count: None,
        internal_provider_request_count_authority:
            "unavailable_from_executor_events; turn count is not a provider-request count"
                .to_string(),
    };
    for (index, line) in source_text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Model request state JSONL {} line {} is invalid: {error}",
                source.display(),
                index + 1
            )
        })?;
        let event_type = event
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if state.thread_id.is_none() {
            state.thread_id = event
                .get("thread_id")
                .or_else(|| event.get("session_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }
        match manifest.runtime.executor {
            crate::agent_token::AgentTokenExecutor::Codex => match event_type {
                "turn.started" | "turn/started" => state.turn_started_count += 1,
                "turn.completed" | "turn/completed" => {
                    state.turn_completed_count += 1;
                    if event.get("usage").is_some() {
                        state.usage_event_count += 1;
                    }
                }
                "turn.failed" | "turn/failed" => state.turn_failed_count += 1,
                "model.rerouted" | "model/rerouted" => state.model_reroute_event_count += 1,
                "model.safety_buffering.updated"
                | "model/safetyBufferingUpdated"
                | "model.safety.updated"
                | "model/safetyUpdated" => state.safety_event_count += 1,
                "error" => state.provider_error_event_count += 1,
                _ => {}
            },
            crate::agent_token::AgentTokenExecutor::Claude => {
                state.model_reroute_event_count += json_type_occurrences(&event, "fallback");
                match event_type {
                    "system"
                        if event.get("subtype").and_then(serde_json::Value::as_str)
                            == Some("init") =>
                    {
                        state.turn_started_count += 1;
                    }
                    "result" => {
                        if event.get("is_error").and_then(serde_json::Value::as_bool) == Some(true)
                        {
                            state.turn_failed_count += 1;
                        } else {
                            state.turn_completed_count += 1;
                        }
                        if event.get("usage").is_some() {
                            state.usage_event_count += 1;
                        }
                    }
                    "error" => state.provider_error_event_count += 1,
                    _ => {}
                }
            }
        }
    }
    state.terminal_state = if state.turn_failed_count > 0 {
        "failed"
    } else if state.turn_completed_count > 0 {
        "completed"
    } else {
        "incomplete"
    }
    .to_string();
    Ok(state)
}

fn json_type_occurrences(value: &serde_json::Value, expected: &str) -> usize {
    match value {
        serde_json::Value::Object(object) => {
            usize::from(object.get("type").and_then(serde_json::Value::as_str) == Some(expected))
                + object
                    .values()
                    .map(|value| json_type_occurrences(value, expected))
                    .sum::<usize>()
        }
        serde_json::Value::Array(values) => values
            .iter()
            .map(|value| json_type_occurrences(value, expected))
            .sum(),
        _ => 0,
    }
}

struct ImportedExecutorUsage {
    usage: crate::NormalizedAgentTokenUsage,
    provider_refusal: bool,
    provider_stop_reason: Option<String>,
}

fn import_executor_usage(
    manifest: &AgentTokenCampaignManifest,
    source: &Path,
    run_id: &str,
    workload_id: &str,
    mode: AgentTokenMode,
) -> Result<ImportedExecutorUsage, String> {
    match manifest.runtime.executor {
        crate::agent_token::AgentTokenExecutor::Codex => import_codex_usage(
            source,
            run_id,
            workload_id,
            mode,
            manifest.accounting_profile,
            &manifest.model,
        )
        .map(|usage| ImportedExecutorUsage {
            usage,
            provider_refusal: false,
            provider_stop_reason: None,
        }),
        crate::agent_token::AgentTokenExecutor::Claude => {
            crate::agent_token::import_claude_usage_with_outcome(
                source,
                run_id,
                workload_id,
                mode,
                manifest.accounting_profile,
                &manifest.model,
                manifest.claude_model_admission,
            )
            .map(|imported| ImportedExecutorUsage {
                usage: imported.usage,
                provider_refusal: imported.provider_refusal,
                provider_stop_reason: Some(imported.provider_stop_reason),
            })
        }
    }
}

fn extract_and_validate_executor_transcript(
    manifest: &AgentTokenCampaignManifest,
    source: &Path,
    run_id: &str,
    mode: AgentTokenMode,
    git_start_state_proof: Option<&AgentTokenGitStartStateProof>,
) -> Result<crate::AgentTokenCommandTranscript, String> {
    let clean_main_head_proven = git_start_state_proof.is_some_and(|proof| proof.passed);
    match manifest.runtime.executor {
        crate::agent_token::AgentTokenExecutor::Codex => {
            extract_and_validate_codex_transcript_with_workflow_options(
                source,
                run_id,
                mode,
                manifest.accounting_profile,
                AgentTokenTranscriptWorkflowOptions {
                    ait_sprint_mode: manifest.ait_sprint_mode,
                    ait_edit_root_mode: (manifest.protocol_revision
                        == AGENT_TOKEN_PROTOCOL_REVISION)
                        .then_some(manifest.ait_edit_root_mode),
                    git_worktree_mode: manifest.git_worktree_mode,
                    clean_main_head_proven,
                },
            )
        }
        crate::agent_token::AgentTokenExecutor::Claude => {
            extract_and_validate_claude_transcript_with_workflow_options(
                source,
                run_id,
                mode,
                manifest.accounting_profile,
                AgentTokenTranscriptWorkflowOptions {
                    ait_sprint_mode: manifest.ait_sprint_mode,
                    ait_edit_root_mode: (manifest.protocol_revision
                        == AGENT_TOKEN_PROTOCOL_REVISION)
                        .then_some(manifest.ait_edit_root_mode),
                    git_worktree_mode: manifest.git_worktree_mode,
                    clean_main_head_proven,
                },
            )
        }
    }
}

fn classify_codex_infrastructure_failure(
    raw_events: &Path,
    stderr: &Path,
    process: &TimedProcessResult,
    transcript: &AgentTokenCommandTranscript,
    usage: Option<&crate::NormalizedAgentTokenUsage>,
) -> Option<String> {
    let spawn_failed = codex_tool_process_spawn_failed(raw_events, stderr);
    let recovered_spawn = spawn_failed
        && !process.timed_out
        && process.exit_code == Some(0)
        && usage.is_some()
        && transcript.valid
        && transcript.errors.is_empty()
        && transcript.command_count > 0;
    if spawn_failed && !recovered_spawn {
        return Some("codex_tool_process_spawn_failure".to_string());
    }
    let source = fs::read_to_string(raw_events).ok()?;
    let mut messages = Vec::new();
    for line in source.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let message = match event.get("type").and_then(serde_json::Value::as_str) {
            Some("error") => event.get("message").and_then(serde_json::Value::as_str),
            Some("turn.failed") => event
                .pointer("/error/message")
                .and_then(serde_json::Value::as_str),
            Some("item.completed")
                if event
                    .pointer("/item/type")
                    .and_then(serde_json::Value::as_str)
                    == Some("error") =>
            {
                event
                    .pointer("/item/message")
                    .and_then(serde_json::Value::as_str)
            }
            _ => continue,
        };
        messages.push(message.unwrap_or_default().to_string());
    }
    if messages.is_empty() {
        return None;
    }
    let combined = messages.join(" ").to_ascii_lowercase();
    let classification =
        if combined.contains("usage limit") || combined.contains("purchase more credits") {
            "provider_usage_limit"
        } else if combined.contains("rate limit") || combined.contains("too many requests") {
            "provider_rate_limit"
        } else if combined.contains("unauthorized")
            || combined.contains("authentication")
            || combined.contains("log in")
        {
            "provider_authentication_failure"
        } else if combined.contains("model")
            && (combined.contains("not found") || combined.contains("unavailable"))
        {
            "provider_model_unavailable"
        } else if combined.contains("reconnecting")
            || combined.contains("stream disconnected")
            || combined.contains("failed to lookup address")
            || combined.contains("connection failed")
            || combined.contains("error sending request")
            || combined.contains("falling back from websockets")
        {
            "provider_transport_failure"
        } else if process.exit_code == Some(0)
            || usage.is_some()
            || transcript.command_count != 0
            || process.timed_out
        {
            "provider_runtime_error_event"
        } else {
            "provider_session_failed_before_candidate_execution"
        };
    Some(classification.to_string())
}

fn codex_tool_process_spawn_failed(raw_events: &Path, stderr: &Path) -> bool {
    let stderr_source = fs::read_to_string(stderr).unwrap_or_default();
    let unified_failure = stderr_source
        .contains("codex_core::tools::router: error=exec_command failed")
        && stderr_source.contains("CreateProcess");
    let legacy_failure = stderr_source.contains("codex_core::exec: exec error:")
        && stderr_source.contains("codex_core::tools::router: error=execution error: Io(");
    if unified_failure || legacy_failure {
        return true;
    }
    let Ok(source) = fs::read_to_string(raw_events) else {
        return false;
    };
    source.lines().any(|line| {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            return false;
        };
        event.get("type").and_then(serde_json::Value::as_str) == Some("item.completed")
            && event
                .pointer("/item/type")
                .and_then(serde_json::Value::as_str)
                == Some("command_execution")
            && event
                .pointer("/item/exit_code")
                .and_then(serde_json::Value::as_i64)
                == Some(-1)
            && event
                .pointer("/item/aggregated_output")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|output| {
                    output.contains("execution error: Io(")
                        && output.contains("No such file or directory")
                })
    })
}

/// Claude-executor counterpart of the infrastructure-failure classifier.
/// Failure signals are the terminal result event (subtype other than
/// success, or is_error), explicit stream error events, and a session that
/// ended without any terminal result event; classification then applies the
/// shared provider vocabulary over the collected messages plus stderr.
fn classify_claude_infrastructure_failure(
    raw_events: &Path,
    stderr: &Path,
    process: &TimedProcessResult,
    transcript: &AgentTokenCommandTranscript,
    usage: Option<&crate::NormalizedAgentTokenUsage>,
) -> Option<String> {
    let source = fs::read_to_string(raw_events).ok()?;
    let mut saw_result = false;
    let mut messages = Vec::new();
    for line in source.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match event.get("type").and_then(serde_json::Value::as_str) {
            Some("result") => {
                saw_result = true;
                let subtype = event
                    .get("subtype")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let is_error = event
                    .get("is_error")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if subtype != "success" || is_error {
                    messages.push(subtype.to_string());
                    if let Some(text) = event.get("result").and_then(serde_json::Value::as_str) {
                        messages.push(text.to_string());
                    }
                    if let Some(text) = event
                        .pointer("/error/message")
                        .and_then(serde_json::Value::as_str)
                    {
                        messages.push(text.to_string());
                    }
                }
            }
            Some("error") => {
                messages.push(
                    event
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                );
            }
            _ => {}
        }
    }
    if !saw_result && !process.timed_out {
        messages.push("session ended without a terminal result event".to_string());
    }
    if messages.is_empty() {
        return None;
    }
    let stderr_text = fs::read_to_string(stderr).unwrap_or_default();
    let combined = format!("{} {}", messages.join(" "), stderr_text).to_ascii_lowercase();
    let classification = if combined.contains("usage limit")
        || combined.contains("session limit")
        || combined.contains("purchase more credits")
        || combined.contains("out of extra usage")
        || combined.contains("credit balance")
    {
        "provider_usage_limit"
    } else if combined.contains("rate limit")
        || combined.contains("rate_limit")
        || combined.contains("too many requests")
        || combined.contains("overloaded")
    {
        "provider_rate_limit"
    } else if combined.contains("unauthorized")
        || combined.contains("authentication")
        || combined.contains("log in")
        || combined.contains("/login")
        || combined.contains("api key")
    {
        "provider_authentication_failure"
    } else if combined.contains("model")
        && (combined.contains("not found") || combined.contains("unavailable"))
    {
        "provider_model_unavailable"
    } else if combined.contains("connection error")
        || combined.contains("connection failed")
        || combined.contains("network error")
        || combined.contains("fetch failed")
        || combined.contains("socket hang up")
        || combined.contains("econn")
        || combined.contains("etimedout")
    {
        "provider_transport_failure"
    } else if process.exit_code == Some(0)
        || usage.is_some()
        || transcript.command_count != 0
        || process.timed_out
    {
        "provider_runtime_error_event"
    } else {
        "provider_session_failed_before_candidate_execution"
    };
    Some(classification.to_string())
}

fn classify_executor_infrastructure_failure(
    manifest: &AgentTokenCampaignManifest,
    raw_events: &Path,
    stderr: &Path,
    process: &TimedProcessResult,
    transcript: &AgentTokenCommandTranscript,
    usage: Option<&crate::NormalizedAgentTokenUsage>,
) -> Option<String> {
    match manifest.runtime.executor {
        crate::agent_token::AgentTokenExecutor::Codex => {
            classify_codex_infrastructure_failure(raw_events, stderr, process, transcript, usage)
        }
        crate::agent_token::AgentTokenExecutor::Claude => {
            classify_claude_infrastructure_failure(raw_events, stderr, process, transcript, usage)
        }
    }
}

/// Secondary metrics are executor-specific at the event layer: Codex counts
/// patch rejections from its stderr router log, while Claude derives patch
/// attempts and rejections from Edit/Write tool_use and tool_result events
/// inside the stream itself.
fn extract_executor_secondary_metrics(
    manifest: &AgentTokenCampaignManifest,
    raw_events: &Path,
    stderr: &Path,
    transcript: &AgentTokenCommandTranscript,
) -> Result<crate::agent_token::AgentTokenSecondaryMetrics, String> {
    match manifest.runtime.executor {
        crate::agent_token::AgentTokenExecutor::Codex => {
            let mut metrics =
                extract_agent_token_secondary_metrics(raw_events, transcript).unwrap_or_default();
            metrics.apply_patch_rejected_attempts = count_rejected_apply_patch_attempts(stderr)?;
            metrics.apply_patch_attempts = metrics
                .file_change_items
                .saturating_add(metrics.apply_patch_rejected_attempts);
            Ok(metrics)
        }
        crate::agent_token::AgentTokenExecutor::Claude => Ok(
            crate::agent_token::extract_agent_token_claude_secondary_metrics(
                raw_events, transcript,
            )
            .unwrap_or_default(),
        ),
    }
}

fn count_rejected_apply_patch_attempts(stderr: &Path) -> Result<usize, String> {
    let source = fs::read_to_string(stderr).map_err(|error| {
        format!(
            "Failed to read Codex stderr metrics source {}: {error}",
            stderr.display()
        )
    })?;
    const ROUTER_ERROR_PREFIX: &str = "codex_core::tools::router: error=";
    Ok(source
        .lines()
        .filter(|line| {
            let Some((log_prefix, router_error)) = line.split_once(ROUTER_ERROR_PREFIX) else {
                return false;
            };
            log_prefix.split_ascii_whitespace().next_back() == Some("ERROR")
                && (router_error.starts_with("apply_patch")
                    || router_error.starts_with("patch rejected:"))
        })
        .count())
}

fn run_acceptance(
    manifest: &AgentTokenCampaignManifest,
    receipt: &crate::GameFixtureReceipt,
    workspace: &Path,
    run_dir: &Path,
) -> Result<serde_json::Value, String> {
    let stdout = run_dir.join("private/acceptance.raw.json");
    let stderr = run_dir.join("private/acceptance.stderr.txt");
    let result = run_process_to_files(
        &manifest.runtime.node_program,
        &[
            receipt.evaluator_path.as_os_str(),
            "--workload".as_ref(),
            receipt.workload_id.as_ref(),
            "--candidate".as_ref(),
            workspace.as_os_str(),
            "--acceptance".as_ref(),
            receipt.acceptance_path.as_os_str(),
        ],
        workspace,
        &stdout,
        &stderr,
        Duration::from_secs(60),
    )?;
    let bytes = fs::read(&stdout).map_err(|error| {
        format!(
            "Failed to read game acceptance report {}: {error}",
            stdout.display()
        )
    })?;
    let mut report = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| {
        format!(
            "Game acceptance evaluator emitted invalid JSON for {}: {error}",
            receipt.workload_id
        )
    })?;
    if result.timed_out || !matches!(result.exit_code, Some(0 | 1)) {
        return Err(format!(
            "Game acceptance evaluator failed as a harness with exit {:?}, timeout={}",
            result.exit_code, result.timed_out
        ));
    }
    if let Some(object) = report.as_object_mut() {
        object.insert("candidate".to_string(), serde_json::json!("workspace"));
        object.insert(
            "_evaluator_exit_code".to_string(),
            serde_json::json!(result.exit_code),
        );
    }
    write_json_new(&run_dir.join("acceptance-report.json"), &report)?;
    Ok(report)
}

fn run_browser_acceptance(
    manifest: &AgentTokenCampaignManifest,
    receipt: &crate::GameFixtureReceipt,
    workspace: &Path,
    run_dir: &Path,
) -> Result<AgentTokenBrowserReport, String> {
    let Some(browser_program) = manifest.runtime.browser_program.as_deref() else {
        let report = AgentTokenBrowserReport {
            contract: AGENT_TOKEN_BROWSER_REPORT_CONTRACT.to_string(),
            workload_id: receipt.workload_id.clone(),
            required_for_equivalent_completion: true,
            status: "unavailable".to_string(),
            desktop_passed: None,
            mobile_passed: None,
            console_errors: None,
            failed_requests: None,
            horizontal_overflow: None,
            notes: vec!["Campaign runtime did not pin a browser program".to_string()],
        };
        write_json_new(&run_dir.join("browser-report.json"), &report)?;
        return Ok(report);
    };
    let stdout = run_dir.join("private/browser.raw.json");
    let stderr = run_dir.join("private/browser.stderr.txt");
    let result = run_process_to_files(
        &manifest.runtime.node_program,
        &[
            receipt.browser_evaluator_path.as_os_str(),
            "--workload".as_ref(),
            receipt.workload_id.as_ref(),
            "--candidate".as_ref(),
            workspace.as_os_str(),
            "--browser".as_ref(),
            browser_program.as_os_str(),
        ],
        workspace,
        &stdout,
        &stderr,
        Duration::from_secs(90),
    )?;
    let report =
        serde_json::from_slice::<AgentTokenBrowserReport>(&fs::read(&stdout).map_err(|error| {
            format!(
                "Failed to read browser acceptance report {}: {error}",
                stdout.display()
            )
        })?)
        .map_err(|error| {
            format!(
                "Browser evaluator emitted invalid JSON for {}: {error}",
                receipt.workload_id
            )
        })?;
    if result.timed_out || !matches!(result.exit_code, Some(0 | 1)) {
        return Err(format!(
            "Browser evaluator failed as a harness with exit {:?}, timeout={}, status={}",
            result.exit_code, result.timed_out, report.status
        ));
    }
    write_json_new(&run_dir.join("browser-report.json"), &report)?;
    Ok(report)
}

fn verify_workflow(
    manifest: &AgentTokenCampaignManifest,
    mode: AgentTokenMode,
    run_id: &str,
    workspace: &Path,
    git_worktree_path: Option<&Path>,
    git_start_state_proof: Option<&AgentTokenGitStartStateProof>,
    managed_worktree_lifecycle: Option<&AgentTokenManagedWorktreeLifecycle>,
) -> Result<AgentTokenWorkflowVerification, String> {
    match mode {
        AgentTokenMode::GitLinearSingleSession => {
            let task_worktree = git_worktree_path.ok_or_else(|| {
                "Git workflow verification is missing its linked-worktree path".to_string()
            })?;
            let status = command_output(
                &manifest.runtime.git_program,
                &["status", "--porcelain"],
                workspace,
            );
            let head = command_output(
                &manifest.runtime.git_program,
                &["rev-parse", "--verify", "HEAD"],
                workspace,
            );
            let pre_merge_head = command_output(
                &manifest.runtime.git_program,
                &["rev-parse", "--verify", "ORIG_HEAD"],
                workspace,
            );
            let candidate_parent = git_start_state_proof.map(|_| {
                command_output(
                    &manifest.runtime.git_program,
                    &["rev-parse", "--verify", "HEAD^"],
                    workspace,
                )
            });
            let merge_ancestry = command_output(
                &manifest.runtime.git_program,
                &["merge-base", "--is-ancestor", "ORIG_HEAD", "HEAD"],
                workspace,
            );
            let current_branch = command_output(
                &manifest.runtime.git_program,
                &["symbolic-ref", "--short", "HEAD"],
                workspace,
            );
            let worktrees = command_output(
                &manifest.runtime.git_program,
                &["worktree", "list", "--porcelain"],
                workspace,
            );
            let temporary_branch = command_output(
                &manifest.runtime.git_program,
                &["branch", "--list", "benchmark-task"],
                workspace,
            );
            let commit_count = command_output(
                &manifest.runtime.git_program,
                &["rev-list", "--count", "HEAD"],
                workspace,
            );
            let mut reasons = Vec::new();
            let workspace_dirty = match status {
                Ok(output) => {
                    let dirty = !output.trim().is_empty();
                    if dirty {
                        reasons
                            .push("Git working tree is dirty after candidate closeout".to_string());
                    }
                    Some(dirty)
                }
                Err(error) => {
                    reasons.push(error);
                    None
                }
            };
            match (&head, &pre_merge_head) {
                (Ok(current), Ok(previous)) if current.trim() != previous.trim() => {}
                (Ok(_), Ok(_)) => reasons
                    .push("Git merge did not advance main from its pre-merge head".to_string()),
                (Err(error), _) | (_, Err(error)) => reasons.push(error.clone()),
            }
            if let Err(error) = merge_ancestry {
                reasons.push(format!(
                    "Git candidate head is not a fast-forward descendant of ORIG_HEAD: {error}"
                ));
            }
            match current_branch {
                Ok(branch) if branch.trim() == "main" => {}
                Ok(branch) => reasons.push(format!(
                    "Git closeout left `{}` checked out instead of `main`",
                    branch.trim()
                )),
                Err(error) => reasons.push(error),
            }
            match worktrees {
                Ok(output) => {
                    let registered = output
                        .lines()
                        .filter(|line| line.starts_with("worktree "))
                        .count();
                    if registered != 1 {
                        reasons.push(format!(
                            "Git closeout retained {registered} registered worktrees; expected only main"
                        ));
                    }
                }
                Err(error) => reasons.push(error),
            }
            match temporary_branch {
                Ok(output) if output.trim().is_empty() => {}
                Ok(_) => reasons
                    .push("Git closeout retained the temporary benchmark-task branch".to_string()),
                Err(error) => reasons.push(error),
            }
            match commit_count {
                Ok(output) => match output.trim().parse::<u64>() {
                    Ok(count) if count >= 2 => {}
                    Ok(count) => reasons.push(format!(
                        "Git main contains {count} commit(s); baseline plus candidate are required"
                    )),
                    Err(error) => reasons.push(format!(
                        "Git rev-list emitted invalid commit count {:?}: {error}",
                        output.trim()
                    )),
                },
                Err(error) => reasons.push(error),
            }
            let git_start_head_oid = git_start_state_proof.and_then(|proof| proof.head_oid.clone());
            let git_pre_merge_head_oid = git_start_state_proof.and_then(|_| {
                pre_merge_head
                    .as_ref()
                    .ok()
                    .map(|value| value.trim().to_string())
            });
            let git_candidate_parent_oid = candidate_parent
                .as_ref()
                .and_then(|result| result.as_ref().ok().map(|value| value.trim().to_string()));
            let git_lineage_matches_start = git_start_state_proof.map(|proof| {
                proof.passed
                    && proof.head_oid.as_ref() == git_pre_merge_head_oid.as_ref()
                    && proof.head_oid.as_ref() == git_candidate_parent_oid.as_ref()
            });
            if protocol_requires_git_start_state_proof(&manifest.protocol_revision)
                && manifest.accounting_profile == AgentTokenAccountingProfile::SteadyStateTaskCost
                && git_start_state_proof.is_none()
            {
                reasons.push(
                    "Steady-state Git workflow lacks its runner-owned start-state proof"
                        .to_string(),
                );
            }
            if let Some(Err(error)) = candidate_parent.as_ref() {
                reasons.push(format!("Failed to prove Git candidate parent: {error}"));
            }
            if git_lineage_matches_start == Some(false) {
                reasons.push(
                    "Git pre-merge head or candidate parent differs from the proven start HEAD"
                        .to_string(),
                );
            }
            if task_worktree.exists() {
                reasons.push(format!(
                    "Git closeout did not remove linked worktree {}",
                    task_worktree.display()
                ));
            }
            if manifest.git_worktree_mode == AgentTokenGitWorktreeMode::CodexAppEquivalentManaged {
                match managed_worktree_lifecycle {
                    Some(lifecycle) => {
                        if lifecycle.contract != AGENT_TOKEN_MANAGED_WORKTREE_LIFECYCLE_CONTRACT
                            || lifecycle.campaign_id != manifest.campaign_id
                            || lifecycle.run_id != run_id
                            || lifecycle.boundary
                                != AgentTokenGitWorktreeMode::CodexAppEquivalentManaged.as_str()
                            || lifecycle.desktop_private_ipc_invoked
                        {
                            reasons.push(
                                "Managed-worktree lifecycle identity or product boundary differs"
                                    .to_string(),
                            );
                        }
                        if lifecycle.worktree_path != task_worktree.display().to_string()
                            || lifecycle.model_started_at.is_none()
                            || lifecycle.model_completed_at.is_none()
                            || lifecycle.closeout_started_at.is_none()
                            || lifecycle.closed_at.is_none()
                            || !lifecycle.worktree_created_before_model
                            || !lifecycle.host_closeout_after_terminal_model_event
                            || lifecycle.worktree_ready_head_oid.as_deref()
                                != Some(lifecycle.starting_head_oid.as_str())
                        {
                            reasons.push(
                                "Managed worktree was not proven ready before the measured model turn"
                                    .to_string(),
                            );
                        }
                        if !lifecycle.host_commit_created
                            || !lifecycle.main_fast_forwarded
                            || !lifecycle.worktree_removed
                            || !lifecycle.closed
                            || !lifecycle.failure_reasons.is_empty()
                            || lifecycle.candidate_head_oid.is_none()
                            || lifecycle.candidate_head_oid != lifecycle.final_main_head_oid
                        {
                            reasons.push(
                                "Managed-worktree host closeout evidence is incomplete".to_string(),
                            );
                        }
                    }
                    None => reasons.push(
                        "Managed Git workflow lacks runner-owned lifecycle evidence".to_string(),
                    ),
                }
            } else if managed_worktree_lifecycle.is_some() {
                reasons.push(
                    "Agent-managed Git workflow unexpectedly carries host lifecycle evidence"
                        .to_string(),
                );
            }
            Ok(AgentTokenWorkflowVerification {
                contract: AGENT_TOKEN_WORKFLOW_VERIFICATION_CONTRACT.to_string(),
                mode,
                closed: reasons.is_empty(),
                workflow_mode: git_workflow_mode_label(manifest.git_worktree_mode).to_string(),
                ait_edit_root_mode: manifest.ait_edit_root_mode,
                git_worktree_mode: manifest.git_worktree_mode,
                sprint_mode: "not_applicable".to_string(),
                sprint_card_path: None,
                sprint_item_ref: None,
                sprint_item_closed: None,
                default_remote_present: false,
                remote_count: None,
                ait_server_configured: false,
                workspace_dirty,
                current_line: None,
                git_start_head_oid,
                git_pre_merge_head_oid,
                git_candidate_parent_oid,
                git_lineage_matches_start,
                reasons,
            })
        }
        AgentTokenMode::AitLinearSingleSession => {
            let config = command_json(
                &manifest.runtime.ait_program,
                &["config", "show", "--json"],
                workspace,
            );
            let status = command_json(
                &manifest.runtime.ait_program,
                &["status", "--json", "--full"],
                workspace,
            );
            let mut reasons = Vec::new();
            if let Err(error) = config.as_ref() {
                reasons.push(error.clone());
            }
            if let Err(error) = status.as_ref() {
                reasons.push(error.clone());
            }
            let workflow_mode = config
                .as_ref()
                .ok()
                .and_then(|value| value.pointer("/workflow_mode/value"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let sprint_mode = config
                .as_ref()
                .ok()
                .and_then(|value| value.pointer("/sprint/value"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let default_remote_present = config
                .as_ref()
                .ok()
                .and_then(|value| value.get("default_remote"))
                .is_some_and(|value| !value.is_null());
            let ait_server_configured = config
                .as_ref()
                .ok()
                .and_then(|value| value.pointer("/agent_runtime/server_url"))
                .is_some_and(|value| !value.is_null());
            let remote_count = status
                .as_ref()
                .ok()
                .and_then(|value| value.get("remote_count"))
                .and_then(serde_json::Value::as_u64);
            let workspace_dirty = status
                .as_ref()
                .ok()
                .and_then(|value| value.get("workspace_dirty"))
                .and_then(serde_json::Value::as_bool);
            let current_line = status
                .as_ref()
                .ok()
                .and_then(|value| value.get("current_line"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            if workflow_mode != "solo_local" {
                reasons.push(format!(
                    "AIT workflow_mode is {workflow_mode}, expected solo_local"
                ));
            }
            let expected_sprint_mode = manifest.ait_sprint_mode.as_str();
            if sprint_mode != expected_sprint_mode {
                reasons.push(format!(
                    "AIT sprint mode is {sprint_mode}, expected {expected_sprint_mode}"
                ));
            }
            let sprint_card_path = (manifest.ait_sprint_mode == AgentTokenAitSprintMode::On)
                .then(|| AIT_SPRINT_CARD_PATH.to_string());
            let sprint_item_ref = (manifest.ait_sprint_mode == AgentTokenAitSprintMode::On)
                .then(|| sprint_item_ref(run_id));
            let sprint_item_closed = sprint_item_ref.as_ref().map(|item_ref| {
                let card_path = workspace.join(AIT_SPRINT_CARD_PATH);
                match fs::read_to_string(&card_path) {
                    Ok(card) => {
                        let plan_ref = sprint_plan_ref(run_id);
                        let has_plan_ref = card.contains(&format!("[plan-ref: {plan_ref}]"));
                        let closed_line = card.lines().any(|line| {
                            line.contains("- [x]") && line.contains(&format!("[ref: {item_ref}]"))
                        });
                        if !has_plan_ref {
                            reasons
                                .push(format!("AIT sprint card lacks exact plan ref {plan_ref}"));
                        }
                        if !closed_line {
                            reasons.push(format!(
                                "AIT sprint card item {item_ref} is missing or not closed"
                            ));
                        }
                        has_plan_ref && closed_line
                    }
                    Err(error) => {
                        reasons.push(format!(
                            "Failed to read AIT sprint card {}: {error}",
                            card_path.display()
                        ));
                        false
                    }
                }
            });
            if default_remote_present {
                reasons.push("AIT default_remote is configured".to_string());
            }
            if remote_count.unwrap_or_default() != 0 {
                reasons.push("AIT remote_count is not zero".to_string());
            }
            if ait_server_configured {
                reasons.push("AIT agent_runtime.server_url is configured".to_string());
            }
            if workspace_dirty != Some(false) {
                reasons.push("AIT target workspace is not clean after local land".to_string());
            }
            if current_line.as_deref() != Some("main") {
                reasons.push("AIT target workspace did not return to logical main".to_string());
            }
            Ok(AgentTokenWorkflowVerification {
                contract: AGENT_TOKEN_WORKFLOW_VERIFICATION_CONTRACT.to_string(),
                mode,
                closed: reasons.is_empty(),
                workflow_mode,
                ait_edit_root_mode: manifest.ait_edit_root_mode,
                git_worktree_mode: manifest.git_worktree_mode,
                sprint_mode,
                sprint_card_path,
                sprint_item_ref,
                sprint_item_closed,
                default_remote_present,
                remote_count,
                ait_server_configured,
                workspace_dirty,
                current_line,
                git_start_head_oid: None,
                git_pre_merge_head_oid: None,
                git_candidate_parent_oid: None,
                git_lineage_matches_start: None,
                reasons,
            })
        }
    }
}

fn validate_solo_local_config(
    config: &serde_json::Value,
    expected_sprint_mode: AgentTokenAitSprintMode,
) -> Result<(), String> {
    let workflow = config
        .pointer("/workflow_mode/value")
        .and_then(serde_json::Value::as_str);
    let sprint = config
        .pointer("/sprint/value")
        .and_then(serde_json::Value::as_str);
    let default_remote = config.get("default_remote");
    let server_url = config.pointer("/agent_runtime/server_url");
    if workflow != Some("solo_local")
        || sprint != Some(expected_sprint_mode.as_str())
        || default_remote.is_some_and(|value| !value.is_null())
        || server_url.is_some_and(|value| !value.is_null())
    {
        return Err(
            format!(
                "AIT bootstrap did not resolve to solo_local, sprint {}, null default_remote, and null server_url",
                expected_sprint_mode.as_str()
            ),
        );
    }
    Ok(())
}

#[derive(Clone)]
struct CapturedVersions {
    codex: String,
    ait: String,
    git: String,
    node: String,
    browser: Option<String>,
}

fn capture_versions(manifest: &AgentTokenCampaignManifest) -> Result<CapturedVersions, String> {
    let versions = CapturedVersions {
        codex: match manifest.runtime.executor {
            crate::agent_token::AgentTokenExecutor::Codex => {
                program_version(&manifest.runtime.codex_program)?
            }
            crate::agent_token::AgentTokenExecutor::Claude => {
                program_version(manifest.runtime.claude_program.as_deref().ok_or_else(|| {
                    "runtime.claude_program is required for the claude executor".to_string()
                })?)?
            }
        },
        ait: program_version(&manifest.runtime.ait_program)?,
        git: program_version(&manifest.runtime.git_program)?,
        node: program_version(&manifest.runtime.node_program)?,
        browser: manifest
            .runtime
            .browser_program
            .as_deref()
            .map(program_version)
            .transpose()?,
    };
    for (label, expected, observed) in [
        (
            "executor",
            manifest.runtime.executor_version.as_deref(),
            Some(versions.codex.as_str()),
        ),
        (
            "AIT",
            manifest.runtime.ait_version.as_deref(),
            Some(versions.ait.as_str()),
        ),
        (
            "Git",
            manifest.runtime.git_version.as_deref(),
            Some(versions.git.as_str()),
        ),
        (
            "Node",
            manifest.runtime.node_version.as_deref(),
            Some(versions.node.as_str()),
        ),
        (
            "browser",
            manifest.runtime.browser_version.as_deref(),
            versions.browser.as_deref(),
        ),
    ] {
        if expected.is_some() && expected != observed {
            return Err(format!(
                "Pinned {label} version drifted: expected {expected:?}, got {observed:?}"
            ));
        }
    }
    Ok(versions)
}

fn program_version(program: &Path) -> Result<String, String> {
    let output = Command::new(program)
        .arg("--version")
        .output()
        .map_err(|error| {
            format!(
                "Failed to launch pinned program {} --version: {error}",
                program.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "Pinned program {} --version exited with {:?}: {}",
            program.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let version = if stdout.is_empty() { stderr } else { stdout };
    if version.is_empty() {
        Err(format!(
            "Pinned program {} emitted no version",
            program.display()
        ))
    } else {
        Ok(version)
    }
}

fn run_checked_event(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    phase: &str,
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    let output = command.output().map_err(|error| {
        format!(
            "Failed to launch bootstrap command {} in {}: {error}",
            program.display(),
            cwd.display()
        )
    })?;
    events.push(ExternalCommandEvent {
        sequence: *sequence,
        phase: phase.to_string(),
        program: program.display().to_string(),
        args: args.iter().map(|value| (*value).to_string()).collect(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    });
    *sequence += 1;
    if !output.status.success() {
        return Err(format!(
            "Bootstrap command {} {:?} failed with {:?}: {}",
            program.display(),
            args,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn run_recorded_event(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    phase: &str,
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<bool, String> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    let output = command.output().map_err(|error| {
        format!(
            "Failed to launch recorded command {} in {}: {error}",
            program.display(),
            cwd.display()
        )
    })?;
    let success = output.status.success();
    events.push(ExternalCommandEvent {
        sequence: *sequence,
        phase: phase.to_string(),
        program: program.display().to_string(),
        args: args.iter().map(|value| (*value).to_string()).collect(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    });
    *sequence += 1;
    Ok(success)
}

fn command_output(program: &Path, args: &[&str], cwd: &Path) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    let output = command.output().map_err(|error| {
        format!(
            "Failed to launch {} {:?} in {}: {error}",
            program.display(),
            args,
            cwd.display()
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "Command {} {:?} failed with {:?}: {}",
            program.display(),
            args,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn command_json(program: &Path, args: &[&str], cwd: &Path) -> Result<serde_json::Value, String> {
    let output = command_output(program, args, cwd)?;
    serde_json::from_str(&output).map_err(|error| {
        format!(
            "Command {} {:?} emitted invalid JSON: {error}",
            program.display(),
            args
        )
    })
}

fn run_process_to_files(
    program: &Path,
    args: &[&std::ffi::OsStr],
    cwd: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
    timeout: Duration,
) -> Result<TimedProcessResult, String> {
    ensure_parent(stdout_path)?;
    ensure_parent(stderr_path)?;
    let stdout = create_new_file(stdout_path)?;
    let stderr = create_new_file(stderr_path)?;
    let start = Instant::now();
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|error| {
        format!(
            "Failed to launch evaluator {} in {}: {error}",
            program.display(),
            cwd.display()
        )
    })?;
    wait_for_child(&mut child, timeout, start)
}

fn wait_for_child(
    child: &mut std::process::Child,
    timeout: Duration,
    start: Instant,
) -> Result<TimedProcessResult, String> {
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Failed to poll benchmark process: {error}"))?
        {
            return Ok(TimedProcessResult {
                exit_code: status.code(),
                timed_out: false,
                elapsed_ms: elapsed_millis(start),
            });
        }
        if start.elapsed() >= timeout {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGTERM);
            }
            let grace = Instant::now();
            while grace.elapsed() < Duration::from_millis(500) {
                if child
                    .try_wait()
                    .map_err(|error| format!("Failed to poll timed-out process: {error}"))?
                    .is_some()
                {
                    return Ok(TimedProcessResult {
                        exit_code: None,
                        timed_out: true,
                        elapsed_ms: elapsed_millis(start),
                    });
                }
                thread::sleep(Duration::from_millis(20));
            }
            child
                .kill()
                .map_err(|error| format!("Failed to kill timed-out benchmark process: {error}"))?;
            let _ = child.wait();
            return Ok(TimedProcessResult {
                exit_code: None,
                timed_out: true,
                elapsed_ms: elapsed_millis(start),
            });
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn elapsed_millis(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn prepare_empty_directory(path: &Path, kind: &str) -> Result<(), String> {
    if path.exists() {
        if !path.is_dir() {
            return Err(format!(
                "{kind} path is not a directory: {}",
                path.display()
            ));
        }
        if fs::read_dir(path)
            .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?
            .next()
            .is_some()
        {
            return Err(format!(
                "{kind} {} must be absent or empty; evidence is never overwritten",
                path.display()
            ));
        }
    } else {
        fs::create_dir_all(path)
            .map_err(|error| format!("Failed to create {}: {error}", path.display()))?;
    }
    Ok(())
}

fn prepare_campaign_output_directory(path: &Path) -> Result<PathBuf, String> {
    prepare_empty_directory(path, "campaign output")?;
    fs::canonicalize(path).map_err(|error| {
        format!(
            "Failed to resolve campaign output directory {}: {error}",
            path.display()
        )
    })
}

fn copy_file_new(source: &Path, destination: &Path) -> Result<(), String> {
    let bytes = fs::read(source)
        .map_err(|error| format!("Failed to read {}: {error}", source.display()))?;
    ensure_parent(destination)?;
    let mut output = create_new_file(destination)?;
    output
        .write_all(&bytes)
        .map_err(|error| format!("Failed to write {}: {error}", destination.display()))
}

fn create_new_file(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "Failed to create {} without overwriting: {error}",
                path.display()
            )
        })
}

fn decode_json_file<T: DeserializeOwned>(path: &Path, kind: &str) -> Result<T, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Failed to read {kind} {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Failed to decode {kind} {}: {error}", path.display()))
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    Ok(())
}

fn write_json_line_new(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut text = serde_json::to_string(value)
        .map_err(|error| format!("Failed to encode JSONL for {}: {error}", path.display()))?;
    text.push('\n');
    write_text_new(path, &text)
}

fn write_json_lines_new<T: Serialize>(path: &Path, values: &[T]) -> Result<(), String> {
    let mut output = String::new();
    for value in values {
        output.push_str(
            &serde_json::to_string(value).map_err(|error| {
                format!("Failed to encode JSONL for {}: {error}", path.display())
            })?,
        );
        output.push('\n');
    }
    write_text_new(path, &output)
}

fn write_command_events(
    path: &Path,
    transcript: &AgentTokenCommandTranscript,
) -> Result<(), String> {
    let events = transcript
        .commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            serde_json::json!({
                "contract": "ait-agent-token-command-event/v1",
                "sequence": index + 1,
                "command": command,
            })
        })
        .collect::<Vec<_>>();
    write_json_lines_new(path, &events)
}

fn resolve_campaign_evidence_dir(campaign_dir: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(campaign_dir).map_err(|error| {
        format!(
            "Failed to resolve campaign evidence directory {}: {error}",
            campaign_dir.display()
        )
    })
}

pub fn validate_agent_token_campaign_evidence(
    manifest: &AgentTokenCampaignManifest,
    campaign_dir: &Path,
) -> Result<Vec<String>, String> {
    let campaign_dir = resolve_campaign_evidence_dir(campaign_dir)?;
    validate_agent_token_campaign_evidence_internal(
        manifest,
        &campaign_dir,
        true,
        campaign_dir
            .join(AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_SELECTION_FILE)
            .is_file(),
        &BTreeSet::new(),
    )
}

/// Revalidates immutable campaign evidence. `exempt_run_ids` skips per-run
/// checks for exactly the lanes an authorized whole-pair recovery is about to
/// replace; without it a provider-truncated lane aborts resume before the
/// recovery contract can reach it. Every other run and every campaign-level
/// check is unaffected, and the set is empty on every path except that
/// authorized recovery.
fn validate_agent_token_campaign_evidence_internal(
    manifest: &AgentTokenCampaignManifest,
    campaign_dir: &Path,
    require_complete: bool,
    allow_host_shutdown_partial: bool,
    exempt_run_ids: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    for required in [
        "campaign-manifest.json",
        "fixture-manifest.json",
        "protocol.json",
        "randomization-schedule.json",
        "executor-preflight-prompt.txt",
        "executor-preflight-report.json",
        "executor-preflight-usage.jsonl",
        "executor-preflight-environment.json",
        "executor-preflight-permission-profile.json",
        "git-worktree-permission-profile.json",
        "git-worktree-permission-preflight-report.json",
        "private/executor-preflight-events.raw.jsonl",
        "private/executor-preflight.stderr.txt",
        "private/git-worktree-permission-preflight-events.jsonl",
        "raw-run-index.json",
        "aggregate-report.json",
        "comparison-report.json",
        "claim-boundary.md",
    ] {
        let path = campaign_dir.join(required);
        let metadata = fs::symlink_metadata(&path);
        if !metadata
            .as_ref()
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        {
            errors.push(format!(
                "campaign is missing regular evidence file {required}"
            ));
        }
    }
    let preflight_prompt =
        fs::read_to_string(campaign_dir.join("executor-preflight-prompt.txt"))
            .map_err(|error| format!("Failed to read executor preflight prompt: {error}"))?;
    if preflight_prompt != EXECUTOR_PREFLIGHT_PROMPT {
        errors.push("executor preflight prompt differs from the compiled contract".to_string());
    }
    let preflight_report = decode_json_file::<AgentTokenExecutorPreflightReport>(
        &campaign_dir.join("executor-preflight-report.json"),
        "executor preflight report",
    )?;
    if preflight_report.contract != AGENT_TOKEN_EXECUTOR_PREFLIGHT_CONTRACT
        || preflight_report.campaign_id != manifest.campaign_id
    {
        errors.push("executor preflight report contract or campaign linkage differs".to_string());
    }
    if preflight_report.required_command_count != AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT {
        errors.push("executor preflight required-command count differs".to_string());
    }
    let git_permission_preflight =
        decode_json_file::<AgentTokenGitWorktreePermissionPreflightReport>(
            &campaign_dir.join("git-worktree-permission-preflight-report.json"),
            "Git worktree permission preflight report",
        )?;
    let git_permission_profile = decode_json_file::<AgentTokenCodexPermissionProfile>(
        &campaign_dir.join("git-worktree-permission-profile.json"),
        "Git worktree permission profile",
    )?;
    if git_permission_preflight.contract != AGENT_TOKEN_GIT_WORKTREE_PERMISSION_PREFLIGHT_CONTRACT
        || git_permission_preflight.campaign_id != manifest.campaign_id
        || git_permission_preflight.permission_profile != git_permission_profile
        || git_permission_preflight.codex_version.trim().is_empty()
        || git_permission_preflight.git_version.trim().is_empty()
        || manifest
            .runtime
            .git_version
            .as_deref()
            .is_some_and(|expected| git_permission_preflight.git_version != expected)
        || git_permission_preflight.required_command_count != 5
        || git_permission_preflight.executed_command_count != 5
        || git_permission_preflight.successful_command_count != 5
        || !git_permission_preflight.main_clean
        || git_permission_preflight.registered_worktree_count != Some(1)
        || !git_permission_preflight.temporary_branch_absent
        || git_permission_preflight.main_commit_count != Some(2)
        || !git_permission_preflight.task_path_absent
        || !git_permission_preflight.passed
        || !git_permission_preflight.failure_reasons.is_empty()
    {
        errors.push("Git worktree permission preflight did not pass exactly".to_string());
    }
    if git_permission_profile.contract != AGENT_TOKEN_CODEX_PERMISSION_PROFILE_CONTRACT
        || git_permission_profile.name != CODEX_PERMISSION_PROFILE_NAME
        || git_permission_profile.extends != CODEX_PERMISSION_PROFILE_PARENT
        || git_permission_profile.network_enabled
        || git_permission_profile.additional_workspace_roots.len() != 2
        || git_permission_profile.git_write_exceptions.len() != 3
    {
        errors.push("Git worktree permission profile differs from the narrow contract".to_string());
    }
    let permission_events = fs::read_to_string(
        campaign_dir.join("private/git-worktree-permission-preflight-events.jsonl"),
    )
    .map_err(|error| format!("Failed to read Git permission-preflight events: {error}"))?;
    let observed_permission_commands = permission_events
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| {
            event.get("phase").and_then(serde_json::Value::as_str)
                == Some("codex-permission-profile-probe")
        })
        .collect::<Vec<_>>();
    if observed_permission_commands.len() != 5
        || observed_permission_commands
            .iter()
            .any(|event| event.get("exit_code").and_then(serde_json::Value::as_i64) != Some(0))
    {
        errors.push("Git permission-preflight raw command evidence differs".to_string());
    }
    if !preflight_report.passed || !preflight_report.failure_reasons.is_empty() {
        errors.push("executor preflight did not pass cleanly".to_string());
    }
    if preflight_report.started_command_count != AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT
        || preflight_report.observed_command_count != AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT
        || preflight_report.distinct_command_count != AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT
        || preflight_report.successful_command_count != AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT
        || preflight_report.failed_command_count != 0
        || preflight_report.unexpected_command_count != 0
        || preflight_report.sequential_violation_count != 0
        || preflight_report.unexpected_tool_item_count != 0
        || preflight_report.file_change_item_count != 0
        || preflight_report.codex_exit_code != Some(0)
        || preflight_report.codex_timed_out
        || preflight_report.infrastructure_failure.is_some()
        || preflight_report.final_workspace_digest.as_deref()
            != Some(preflight_report.initial_workspace_digest.as_str())
        || preflight_report
            .usage
            .as_ref()
            .is_none_or(|usage| usage.completed_turns != 1)
    {
        errors.push("executor preflight admission fields are inconsistent".to_string());
    }
    let preflight_environment = decode_json_file::<AgentTokenExecutorPreflightEnvironment>(
        &campaign_dir.join("executor-preflight-environment.json"),
        "executor preflight environment",
    )?;
    let executor_permission_profile = decode_json_file::<AgentTokenCodexPermissionProfile>(
        &campaign_dir.join("executor-preflight-permission-profile.json"),
        "executor preflight permission profile",
    )?;
    if preflight_environment.contract != AGENT_TOKEN_EXECUTOR_PREFLIGHT_ENVIRONMENT_CONTRACT
        || preflight_environment.model != manifest.model
        || preflight_environment.network_policy != manifest.network_policy
        || preflight_environment.project_doc_max_bytes != manifest.runtime.project_doc_max_bytes
        || preflight_environment.sandbox != CODEX_PERMISSION_PROFILE_LABEL
        || preflight_environment.codex_permission_profile != CODEX_PERMISSION_PROFILE_NAME
        || preflight_environment.codex_permission_profile_parent != CODEX_PERMISSION_PROFILE_PARENT
        || preflight_environment.codex_version.trim().is_empty()
        || manifest
            .runtime
            .executor_version
            .as_deref()
            .is_some_and(|expected| preflight_environment.codex_version != expected)
        || preflight_environment.benchmark_enabled_feature_overrides
            != executor_enabled_feature_overrides(manifest)
        || preflight_environment.benchmark_disabled_feature_overrides
            != executor_disabled_feature_overrides(manifest)
    {
        errors.push("executor preflight environment pin differs".to_string());
    }
    if executor_permission_profile.contract != AGENT_TOKEN_CODEX_PERMISSION_PROFILE_CONTRACT
        || executor_permission_profile.name != CODEX_PERMISSION_PROFILE_NAME
        || executor_permission_profile.extends != CODEX_PERMISSION_PROFILE_PARENT
        || executor_permission_profile.network_enabled
        || !executor_permission_profile
            .additional_workspace_roots
            .is_empty()
        || !executor_permission_profile.git_write_exceptions.is_empty()
    {
        errors.push("executor preflight permission profile differs".to_string());
    }
    let preflight_usage_source =
        fs::read_to_string(campaign_dir.join("executor-preflight-usage.jsonl"))
            .map_err(|error| format!("Failed to read executor preflight usage: {error}"))?;
    let preflight_usage = preflight_usage_source
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<AgentTokenExecutorPreflightUsage>)
        .transpose()
        .map_err(|error| format!("Failed to decode executor preflight usage: {error}"))?;
    if preflight_usage != preflight_report.usage {
        errors.push("executor preflight usage evidence differs from its report".to_string());
    }
    if preflight_usage.as_ref().is_none_or(|usage| {
        usage.contract != AGENT_TOKEN_EXECUTOR_PREFLIGHT_USAGE_CONTRACT
            || usage.model_provider != manifest.model.provider
            || usage.model_id != manifest.model.model_id
            || usage.model_revision != manifest.model.model_revision
            || usage.reasoning_effort != manifest.model.reasoning_effort
            || usage.completed_turns != 1
    }) {
        errors.push("executor preflight provider/model usage pin differs".to_string());
    }
    if protocol_requires_claude_model_evidence(&manifest.protocol_revision)
        && manifest.runtime.executor == crate::agent_token::AgentTokenExecutor::Claude
    {
        match import_executor_usage(
            manifest,
            &campaign_dir.join("private/executor-preflight-events.raw.jsonl"),
            &format!("{}-executor-preflight", manifest.campaign_id),
            "executor-preflight",
            AgentTokenMode::GitLinearSingleSession,
        ) {
            Ok(imported)
                if !imported.provider_refusal
                    && imported.provider_stop_reason.as_deref() == Some("end_turn")
                    && preflight_usage.as_ref()
                        == Some(&preflight_usage_from_normalized(&imported.usage)) => {}
            Ok(_) => errors.push(
                "executor preflight private Claude model-purity or terminal outcome evidence differs"
                    .to_string(),
            ),
            Err(error) => errors.push(format!(
                "executor preflight private Claude model-purity evidence is invalid: {error}"
            )),
        }
    }
    // Claude preflight streams carry no Codex item events, so the pinned
    // executor's parser must be used here exactly as the initial run does.
    // Calling the Codex-specific inspection made every counter read zero and
    // rendered every Claude campaign unresumable.
    let observed_preflight = inspect_executor_preflight_events_for(
        manifest,
        &campaign_dir.join("private/executor-preflight-events.raw.jsonl"),
    );
    if observed_preflight.started_command_count != preflight_report.started_command_count
        || observed_preflight.observed_command_count != preflight_report.observed_command_count
        || observed_preflight.distinct_command_count != preflight_report.distinct_command_count
        || observed_preflight.successful_command_count != preflight_report.successful_command_count
        || observed_preflight.failed_command_count != preflight_report.failed_command_count
        || observed_preflight.unexpected_command_count != preflight_report.unexpected_command_count
        || observed_preflight.sequential_violation_count
            != preflight_report.sequential_violation_count
        || observed_preflight.unexpected_tool_item_count
            != preflight_report.unexpected_tool_item_count
        || observed_preflight.file_change_item_count != preflight_report.file_change_item_count
        || !observed_preflight.errors.is_empty()
    {
        errors.push("executor preflight raw-event evidence differs from its report".to_string());
    }
    if codex_tool_process_spawn_failed(
        &campaign_dir.join("private/executor-preflight-events.raw.jsonl"),
        &campaign_dir.join("private/executor-preflight.stderr.txt"),
    ) {
        errors.push("executor preflight contains a command-process spawn failure".to_string());
    }
    let schedule =
        crate::load_agent_token_schedule(&campaign_dir.join("randomization-schedule.json"))?;
    let runs = if allow_host_shutdown_partial {
        load_agent_token_run_summaries_with_allowed_missing(
            campaign_dir,
            Some(crate::AGENT_TOKEN_HOST_SHUTDOWN_INTERRUPTED_RUN_ID),
        )?
    } else {
        crate::load_agent_token_run_summaries(campaign_dir)?
    };
    if schedule.contract != crate::AGENT_TOKEN_SCHEDULE_CONTRACT {
        errors.push("schedule contract is unsupported".to_string());
    }
    if schedule.campaign_id != manifest.campaign_id {
        errors.push("schedule campaign_id does not match manifest".to_string());
    }
    if schedule.protocol_revision != manifest.protocol_revision {
        errors.push("schedule protocol_revision does not match manifest".to_string());
    }
    if schedule.entry_count != schedule.entries.len() {
        errors.push("schedule entry_count does not match entries".to_string());
    }
    if schedule.entries.len() % 2 != 0
        || schedule.entries.chunks_exact(2).any(|pair| {
            pair[0].workload_id != pair[1].workload_id
                || pair[0].attempt != pair[1].attempt
                || pair.iter().map(|entry| entry.mode).collect::<BTreeSet<_>>()
                    != BTreeSet::from([
                        AgentTokenMode::GitLinearSingleSession,
                        AgentTokenMode::AitLinearSingleSession,
                    ])
        })
    {
        errors.push("schedule does not preserve adjacent atomic Git/AIT pairs".to_string());
    }
    // Whole-pair recovery already classified the exact trailing contaminated
    // pair and supplied only those two run IDs as exemptions. Building the
    // ordinary statistical view here would reject that same unadjudicated
    // infrastructure lane before recovery can replace it. Every non-exempt
    // run is still validated below, and complete validation never accepts an
    // exemption.
    if exempt_run_ids.is_empty() {
        let statistical_view = if allow_host_shutdown_partial {
            load_agent_token_campaign_statistical_view_allowing_host_shutdown_partial(
                manifest,
                &schedule,
                campaign_dir,
            )
        } else {
            load_agent_token_campaign_statistical_view(manifest, &schedule, campaign_dir)
        };
        match statistical_view {
            Ok(view) if require_complete && view.effective_runs.len() != schedule.entries.len() => {
                errors.push(format!(
                    "observed {} statistically admitted run summaries for {} scheduled entries ({} raw source summaries)",
                    view.effective_runs.len(),
                    schedule.entries.len(),
                    runs.len()
                ));
            }
            Ok(_) => {}
            Err(error) => errors.push(format!(
                "campaign statistical view failed validation: {error}"
            )),
        }
    } else if require_complete {
        errors.push("Complete evidence validation cannot exempt recovered run IDs".to_string());
    }
    let mut seen = BTreeSet::new();
    for run in &runs {
        if exempt_run_ids.contains(&run.run_id) {
            continue;
        }
        if run.contract != AGENT_TOKEN_RUN_SUMMARY_CONTRACT {
            errors.push(format!("run {} contract is unsupported", run.run_id));
        }
        if run.campaign_id != manifest.campaign_id {
            errors.push(format!("run {} campaign linkage differs", run.run_id));
        }
        if run.accounting_profile != manifest.accounting_profile {
            errors.push(format!("run {} accounting profile differs", run.run_id));
        }
        if run.transcript.accounting_profile != manifest.accounting_profile {
            errors.push(format!(
                "run {} transcript accounting profile differs",
                run.run_id
            ));
        }
        if !seen.insert(run.run_id.as_str()) {
            errors.push(format!("duplicate run summary id: {}", run.run_id));
        }
        let Some(entry) = schedule
            .entries
            .iter()
            .find(|entry| entry.run_id == run.run_id)
        else {
            errors.push(format!("run {} is absent from schedule", run.run_id));
            continue;
        };
        if entry.workload_id != run.workload_id
            || entry.mode != run.mode
            || entry.attempt != run.attempt
            || entry.block_index != run.block_index
            || entry.randomized_order != run.randomized_order
        {
            errors.push(format!("run {} metadata differs from schedule", run.run_id));
        }
        let run_dir = campaign_dir.join("runs").join(&run.run_id);
        for required in [
            "campaign-manifest.json",
            "codex-permission-profile.json",
            "fixture-manifest.json",
            "prompt.txt",
            "run-manifest.json",
            "provider-usage.jsonl",
            "command-events.jsonl",
            "acceptance-report.json",
            "browser-report.json",
            "environment.json",
            "workflow-verification.json",
            "run-summary.json",
        ] {
            let path = run_dir.join(required);
            let metadata = fs::symlink_metadata(&path);
            if !metadata
                .as_ref()
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            {
                errors.push(format!(
                    "run {} is missing regular evidence file {required}",
                    run.run_id
                ));
            }
        }
        if manifest.protocol_revision == AGENT_TOKEN_PROTOCOL_REVISION {
            for required in ["model-request-state.json", "private/codex-events.raw.jsonl"] {
                let path = run_dir.join(required);
                let metadata = fs::symlink_metadata(&path);
                if !metadata
                    .as_ref()
                    .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                {
                    errors.push(format!(
                        "run {} is missing regular model-state evidence file {required}",
                        run.run_id
                    ));
                }
            }
        }
        let managed_git_run = run.mode == AgentTokenMode::GitLinearSingleSession
            && manifest.git_worktree_mode == AgentTokenGitWorktreeMode::CodexAppEquivalentManaged;
        if managed_git_run {
            for required in [
                "managed-worktree-lifecycle.json",
                "private/managed-worktree-closeout-events.jsonl",
            ] {
                let path = run_dir.join(required);
                let metadata = fs::symlink_metadata(&path);
                if !metadata
                    .as_ref()
                    .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                {
                    errors.push(format!(
                        "run {} is missing regular managed-worktree evidence file {required}",
                        run.run_id
                    ));
                }
            }
        }
        let git_start_proof_required =
            protocol_requires_git_start_state_proof(&manifest.protocol_revision)
                && manifest.accounting_profile == AgentTokenAccountingProfile::SteadyStateTaskCost
                && run.mode == AgentTokenMode::GitLinearSingleSession;
        let git_start_proof = git_start_proof_required.then(|| {
            decode_json_file::<AgentTokenGitStartStateProof>(
                &run_dir.join("git-start-state-proof.json"),
                "Git start-state proof",
            )
        });
        match git_start_proof.as_ref() {
            Some(Ok(proof))
                if proof.contract == AGENT_TOKEN_GIT_START_STATE_PROOF_CONTRACT
                    && proof.campaign_id == manifest.campaign_id
                    && proof.run_id == run.run_id
                    && !proof.captured_at.trim().is_empty()
                    && proof.current_branch.as_deref() == Some("main")
                    && proof.head_oid.as_deref().is_some_and(|oid| !oid.is_empty())
                    && proof.head_oid == proof.main_oid
                    && proof.status_porcelain.as_deref() == Some("")
                    && proof.clean
                    && proof.head_matches_main
                    && proof.passed
                    && proof.failure_reasons.is_empty() => {}
            Some(Ok(_)) => errors.push(format!(
                "run {} Git start-state proof did not prove a clean main HEAD",
                run.run_id
            )),
            Some(Err(error)) => errors.push(error.clone()),
            None => {}
        }
        let run_manifest_path = run_dir.join("run-manifest.json");
        let run_permission_profile = decode_json_file::<AgentTokenCodexPermissionProfile>(
            &run_dir.join("codex-permission-profile.json"),
            "run Codex permission profile",
        );
        match run_permission_profile.as_ref() {
            Ok(profile)
                if codex_run_permission_profile_matches(
                    profile,
                    run.mode,
                    manifest.git_worktree_mode,
                ) => {}
            Ok(_) => errors.push(format!(
                "run {} Codex permission profile differs",
                run.run_id
            )),
            Err(error) => errors.push(error.clone()),
        }
        if run_manifest_path.is_file() {
            match fs::read(&run_manifest_path)
                .map_err(|error| format!("Failed to read {}: {error}", run_manifest_path.display()))
                .and_then(|bytes| {
                    serde_json::from_slice::<AgentTokenRunManifest>(&bytes).map_err(|error| {
                        format!("Failed to decode {}: {error}", run_manifest_path.display())
                    })
                }) {
                Ok(run_manifest) => {
                    let expected_git_start_proof =
                        git_start_proof_required.then(|| "git-start-state-proof.json".to_string());
                    if run_manifest.git_start_state_proof != expected_git_start_proof {
                        errors.push(format!(
                            "run {} Git start-state proof linkage differs",
                            run.run_id
                        ));
                    }
                    let expected_execution_workspace = if managed_git_run {
                        "git-worktree-runtime/git-task-worktree"
                    } else {
                        "workspace"
                    };
                    let expected_workflow_mode = match run.mode {
                        AgentTokenMode::GitLinearSingleSession => {
                            git_workflow_mode_label(manifest.git_worktree_mode)
                        }
                        AgentTokenMode::AitLinearSingleSession => "solo_local",
                    };
                    if run_manifest.ait_edit_root_mode != manifest.ait_edit_root_mode
                        || run_manifest.git_worktree_mode != manifest.git_worktree_mode
                        || !run_manifest_execution_workspace_matches(
                            &manifest.protocol_revision,
                            &run_manifest.execution_workspace,
                            expected_execution_workspace,
                        )
                        || run_manifest.workflow_mode != expected_workflow_mode
                    {
                        errors.push(format!(
                            "run {} edit-root, Git-worktree ownership, or execution-root evidence differs",
                            run.run_id
                        ));
                    }
                    let expected_sprint_mode = match run.mode {
                        AgentTokenMode::GitLinearSingleSession => "not_applicable",
                        AgentTokenMode::AitLinearSingleSession => manifest.ait_sprint_mode.as_str(),
                    };
                    let expected_sprint_card = (run.mode == AgentTokenMode::AitLinearSingleSession
                        && manifest.ait_sprint_mode == AgentTokenAitSprintMode::On)
                        .then(|| AIT_SPRINT_CARD_PATH.to_string());
                    let expected_sprint_item = (run.mode == AgentTokenMode::AitLinearSingleSession
                        && manifest.ait_sprint_mode == AgentTokenAitSprintMode::On)
                        .then(|| sprint_item_ref(&run.run_id));
                    if run_manifest.sprint_mode != expected_sprint_mode
                        || run_manifest.sprint_card_path != expected_sprint_card
                        || run_manifest.sprint_item_ref != expected_sprint_item
                    {
                        errors.push(format!(
                            "run {} sprint workflow manifest evidence differs",
                            run.run_id
                        ));
                    }
                    if run_manifest.project_doc_max_bytes != manifest.runtime.project_doc_max_bytes
                        || run_manifest.project_document_loading
                            != project_document_loading_label(
                                manifest.runtime.project_doc_max_bytes,
                            )
                    {
                        errors.push(format!(
                            "run {} project-document loading evidence differs",
                            run.run_id
                        ));
                    }
                    if run_manifest.benchmark_enabled_feature_overrides
                        != executor_enabled_feature_overrides(manifest)
                        || run_manifest.benchmark_disabled_feature_overrides
                            != executor_disabled_feature_overrides(manifest)
                    {
                        errors.push(format!(
                            "run {} Codex feature-override evidence differs",
                            run.run_id
                        ));
                    }
                    if run_manifest.codex_permission_profile != CODEX_PERMISSION_PROFILE_NAME
                        || run_manifest.codex_permission_profile_parent
                            != CODEX_PERMISSION_PROFILE_PARENT
                        || run_permission_profile.as_ref().is_ok_and(|profile| {
                            run_manifest.codex_permission_profile != profile.name
                                || run_manifest.codex_permission_profile_parent != profile.extends
                        })
                    {
                        errors.push(format!(
                            "run {} permission-profile manifest evidence differs",
                            run.run_id
                        ));
                    }
                }
                Err(error) => errors.push(error),
            }
        }
        match decode_json_file::<AgentTokenWorkflowVerification>(
            &run_dir.join("workflow-verification.json"),
            "workflow verification",
        ) {
            Ok(workflow) => {
                if workflow.contract != AGENT_TOKEN_WORKFLOW_VERIFICATION_CONTRACT
                    || workflow.mode != run.mode
                    || workflow.closed != run.workflow_closed
                    || workflow.ait_edit_root_mode != manifest.ait_edit_root_mode
                    || workflow.git_worktree_mode != manifest.git_worktree_mode
                {
                    errors.push(format!(
                        "run {} workflow verification linkage differs",
                        run.run_id
                    ));
                }
                if run.mode == AgentTokenMode::GitLinearSingleSession
                    && workflow.workflow_mode != git_workflow_mode_label(manifest.git_worktree_mode)
                {
                    errors.push(format!(
                        "run {} Git workflow ownership label differs",
                        run.run_id
                    ));
                }
                if run.mode == AgentTokenMode::AitLinearSingleSession {
                    let expected_sprint_card = (manifest.ait_sprint_mode
                        == AgentTokenAitSprintMode::On)
                        .then(|| AIT_SPRINT_CARD_PATH.to_string());
                    let expected_sprint_item = (manifest.ait_sprint_mode
                        == AgentTokenAitSprintMode::On)
                        .then(|| sprint_item_ref(&run.run_id));
                    let expected_closed = (manifest.ait_sprint_mode == AgentTokenAitSprintMode::On)
                        .then_some(!run.provider_refusal);
                    if workflow.workflow_mode != "solo_local"
                        || workflow.sprint_mode != manifest.ait_sprint_mode.as_str()
                        || workflow.sprint_card_path != expected_sprint_card
                        || workflow.sprint_item_ref != expected_sprint_item
                        || workflow.sprint_item_closed != expected_closed
                    {
                        errors.push(format!(
                            "run {} sprint workflow verification differs",
                            run.run_id
                        ));
                    }
                }
                if git_start_proof_required {
                    let proven_start = git_start_proof
                        .as_ref()
                        .and_then(|proof| proof.as_ref().ok())
                        .and_then(|proof| proof.head_oid.as_ref());
                    let linkage_differs = if run.provider_refusal {
                        workflow.git_start_head_oid.as_ref() != proven_start
                    } else {
                        workflow.git_start_head_oid.as_ref() != proven_start
                            || workflow.git_pre_merge_head_oid.as_ref() != proven_start
                            || workflow.git_candidate_parent_oid.as_ref() != proven_start
                            || workflow.git_lineage_matches_start != Some(true)
                    };
                    if linkage_differs {
                        errors.push(format!(
                            "run {} Git closeout lineage differs from its proven start HEAD",
                            run.run_id
                        ));
                    }
                }
            }
            Err(error) => errors.push(error),
        }
        if managed_git_run {
            match decode_json_file::<AgentTokenManagedWorktreeLifecycle>(
                &run_dir.join("managed-worktree-lifecycle.json"),
                "managed-worktree lifecycle",
            ) {
                Ok(lifecycle) => {
                    let proven_start = git_start_proof
                        .as_ref()
                        .and_then(|proof| proof.as_ref().ok())
                        .and_then(|proof| proof.head_oid.as_deref());
                    if lifecycle.contract != AGENT_TOKEN_MANAGED_WORKTREE_LIFECYCLE_CONTRACT
                        || lifecycle.campaign_id != manifest.campaign_id
                        || lifecycle.run_id != run.run_id
                        || lifecycle.boundary
                            != AgentTokenGitWorktreeMode::CodexAppEquivalentManaged.as_str()
                        || lifecycle.desktop_private_ipc_invoked
                        || lifecycle.worktree_path
                            != run_dir
                                .join("git-worktree-runtime/git-task-worktree")
                                .display()
                                .to_string()
                        || proven_start != Some(lifecycle.starting_head_oid.as_str())
                        || lifecycle.worktree_ready_head_oid.as_deref() != proven_start
                        || lifecycle.model_started_at.is_none()
                        || lifecycle.model_completed_at.is_none()
                        || lifecycle.model_elapsed_ms != Some(run.elapsed_ms)
                        || lifecycle.closeout_started_at.is_none()
                        || lifecycle.closed_at.is_none()
                        || lifecycle.closeout_elapsed_ms.is_none()
                        || !lifecycle.worktree_created_before_model
                        || !lifecycle.host_closeout_after_terminal_model_event
                        || !lifecycle.host_commit_created
                        || !lifecycle.main_fast_forwarded
                        || !lifecycle.worktree_removed
                        || !lifecycle.closed
                        || !lifecycle.failure_reasons.is_empty()
                        || lifecycle.candidate_head_oid.is_none()
                        || lifecycle.candidate_head_oid != lifecycle.final_main_head_oid
                        || run.secondary_metrics.host_worktree_provisioning_elapsed_ms
                            != Some(lifecycle.provisioning_elapsed_ms)
                        || run.secondary_metrics.host_worktree_closeout_elapsed_ms
                            != lifecycle.closeout_elapsed_ms
                    {
                        errors.push(format!(
                            "run {} managed-worktree lifecycle evidence differs",
                            run.run_id
                        ));
                    }
                }
                Err(error) => errors.push(error),
            }
        } else if run
            .secondary_metrics
            .host_worktree_provisioning_elapsed_ms
            .is_some()
            || run
                .secondary_metrics
                .host_worktree_closeout_elapsed_ms
                .is_some()
        {
            errors.push(format!(
                "run {} carries host-worktree timing outside a managed Git lane",
                run.run_id
            ));
        }
        if manifest.protocol_revision == AGENT_TOKEN_PROTOCOL_REVISION {
            let raw_events = run_dir.join("private/codex-events.raw.jsonl");
            let recorded = decode_json_file::<AgentTokenModelRequestState>(
                &run_dir.join("model-request-state.json"),
                "model request state",
            );
            let recomputed = inspect_model_request_state(manifest, &raw_events, &run.run_id);
            match (recorded, recomputed) {
                (Ok(recorded), Ok(recomputed)) => {
                    if recorded != recomputed
                        || recorded.contract != AGENT_TOKEN_MODEL_REQUEST_STATE_CONTRACT
                        || recorded.run_id != run.run_id
                        || recorded.executor != manifest.runtime.executor.as_str()
                        || recorded.thread_id.is_none()
                        || recorded.turn_started_count != 1
                        || recorded.turn_completed_count + recorded.turn_failed_count != 1
                        || recorded.usage_event_count != 1
                        || recorded.terminal_state == "incomplete"
                        || recorded.internal_provider_request_count.is_some()
                        || recorded.internal_provider_request_count_authority
                            != "unavailable_from_executor_events; turn count is not a provider-request count"
                    {
                        errors.push(format!(
                            "run {} model-request state is incomplete or differs from raw executor events",
                            run.run_id
                        ));
                    }
                }
                (Err(error), _) | (_, Err(error)) => errors.push(error),
            }
        }
        if let Some(usage) = &run.usage {
            if usage.contract != crate::AGENT_TOKEN_USAGE_CONTRACT
                || usage.run_id != run.run_id
                || usage.workload_id != run.workload_id
                || usage.mode != run.mode
                || usage.accounting_profile != manifest.accounting_profile
            {
                errors.push(format!(
                    "run {} normalized usage linkage differs",
                    run.run_id
                ));
            }
        }
        if protocol_requires_claude_model_evidence(&manifest.protocol_revision)
            && manifest.runtime.executor == crate::agent_token::AgentTokenExecutor::Claude
        {
            let raw_events = run_dir.join("private/codex-events.raw.jsonl");
            let metadata = fs::symlink_metadata(&raw_events);
            if !metadata
                .as_ref()
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            {
                errors.push(format!(
                    "run {} is missing regular private Claude model-purity evidence",
                    run.run_id
                ));
            } else {
                match import_executor_usage(
                    manifest,
                    &raw_events,
                    &run.run_id,
                    &run.workload_id,
                    run.mode,
                ) {
                    Ok(imported) => {
                        if run.usage.as_ref() != Some(&imported.usage)
                            || run.provider_refusal != imported.provider_refusal
                            || run.provider_stop_reason != imported.provider_stop_reason
                        {
                            errors.push(format!(
                                "run {} normalized usage, model purity, or provider outcome differs from private Claude evidence",
                                run.run_id
                            ));
                        }
                    }
                    Err(error) => errors.push(format!(
                        "run {} private Claude model-purity evidence is invalid: {error}",
                        run.run_id
                    )),
                }
            }
            if run.provider_refusal {
                if !run.valid_attempt
                    || run.accepted_equivalent
                    || run.infrastructure_failure.is_some()
                    || run.provider_stop_reason.as_deref() != Some("refusal")
                    || !run
                        .failure_reasons
                        .iter()
                        .any(|reason| reason == "provider returned a refusal")
                {
                    errors.push(format!(
                        "run {} Claude refusal is not retained as a valid, unaccepted functional outcome",
                        run.run_id
                    ));
                }
            } else if run.provider_stop_reason.as_deref() != Some("end_turn") {
                errors.push(format!(
                    "run {} Claude terminal stop reason is not end_turn",
                    run.run_id
                ));
            }
        }
        if run.secondary_metrics.apply_patch_attempts
            != run
                .secondary_metrics
                .file_change_items
                .saturating_add(run.secondary_metrics.apply_patch_rejected_attempts)
        {
            errors.push(format!(
                "run {} apply-patch attempt evidence is inconsistent",
                run.run_id
            ));
        }
        if run.browser.contract != AGENT_TOKEN_BROWSER_REPORT_CONTRACT
            || run.browser.workload_id != run.workload_id
        {
            errors.push(format!(
                "run {} browser evidence linkage differs",
                run.run_id
            ));
        }
        let environment = run_dir.join("environment.json");
        if environment.is_file() {
            match fs::read(&environment)
                .map_err(|error| format!("Failed to read {}: {error}", environment.display()))
                .and_then(|bytes| {
                    serde_json::from_slice::<AgentTokenEnvironment>(&bytes).map_err(|error| {
                        format!("Failed to decode {}: {error}", environment.display())
                    })
                }) {
                Ok(environment) => {
                    if environment.ait_edit_root_mode != manifest.ait_edit_root_mode
                        || environment.git_worktree_mode != manifest.git_worktree_mode
                    {
                        errors.push(format!(
                            "run {} environment edit-root or Git-worktree ownership differs",
                            run.run_id
                        ));
                    }
                    if environment.project_doc_max_bytes != manifest.runtime.project_doc_max_bytes {
                        errors.push(format!(
                            "run {} environment project-document limit differs",
                            run.run_id
                        ));
                    }
                    if environment.benchmark_enabled_feature_overrides
                        != executor_enabled_feature_overrides(manifest)
                        || environment.benchmark_disabled_feature_overrides
                            != executor_disabled_feature_overrides(manifest)
                    {
                        errors.push(format!(
                            "run {} environment Codex feature-override evidence differs",
                            run.run_id
                        ));
                    }
                    if environment.codex_version != preflight_environment.codex_version {
                        errors.push(format!(
                            "run {} Codex version differs from executor preflight",
                            run.run_id
                        ));
                    }
                    if manifest
                        .runtime
                        .ait_version
                        .as_deref()
                        .is_some_and(|expected| environment.ait_version != expected)
                        || manifest
                            .runtime
                            .git_version
                            .as_deref()
                            .is_some_and(|expected| environment.git_version != expected)
                        || manifest
                            .runtime
                            .node_version
                            .as_deref()
                            .is_some_and(|expected| environment.node_version != expected)
                        || manifest
                            .runtime
                            .browser_version
                            .as_deref()
                            .is_some_and(|expected| {
                                environment.browser_version.as_deref() != Some(expected)
                            })
                    {
                        errors.push(format!(
                            "run {} environment tool-version pins differ",
                            run.run_id
                        ));
                    }
                    if environment.codex_permission_profile != CODEX_PERMISSION_PROFILE_NAME
                        || environment.codex_permission_profile_parent
                            != CODEX_PERMISSION_PROFILE_PARENT
                    {
                        errors.push(format!(
                            "run {} environment permission profile differs",
                            run.run_id
                        ));
                    }
                    if run.mode == AgentTokenMode::AitLinearSingleSession
                        && (environment.workflow_mode != "solo_local"
                            || environment.sprint_mode != manifest.ait_sprint_mode.as_str()
                            || environment.ait_server_connected)
                    {
                        errors.push(format!(
                            "run {} environment violates solo_local/sprint-{}/no-server",
                            run.run_id,
                            manifest.ait_sprint_mode.as_str()
                        ));
                    }
                }
                Err(error) => errors.push(error),
            }
        }
    }
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest() -> AgentTokenCampaignManifest {
        AgentTokenCampaignManifest {
            contract: crate::AGENT_TOKEN_CAMPAIGN_CONTRACT.to_string(),
            campaign_id: "test".to_string(),
            protocol_revision: crate::AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
            campaign_scope: crate::AgentTokenCampaignScope::Smoke,
            accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            ait_sprint_mode: AgentTokenAitSprintMode::Off,
            ait_edit_root_mode: crate::agent_token::AgentTokenAitEditRootMode::Explicit,
            git_worktree_mode: AgentTokenGitWorktreeMode::AgentManaged,
            claude_model_admission: crate::agent_token::AgentTokenClaudeModelAdmission::Strict,
            functional_replacement_policy: crate::AgentTokenFunctionalReplacementPolicy::None,
            seed: 1,
            attempts_per_cell: 1,
            workload_ids: vec!["GD-01".to_string()],
            modes: vec![
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ],
            model: crate::AgentTokenModelPin {
                provider: "openai".to_string(),
                model_id: "test-model".to_string(),
                model_revision: "test".to_string(),
                reasoning_effort: "medium".to_string(),
            },
            runtime: crate::AgentTokenRuntime {
                executor: crate::agent_token::AgentTokenExecutor::default(),
                claude_program: None,
                executor_version: None,
                ait_version: None,
                git_version: None,
                node_version: None,
                browser_version: None,
                codex_program: PathBuf::from("codex"),
                ait_program: PathBuf::from("ait"),
                git_program: PathBuf::from("git"),
                node_program: PathBuf::from("node"),
                browser_program: None,
                fixture_manifest: PathBuf::from("fixture.json"),
                run_timeout_seconds: 60,
                ait_first_use_worktree_add_dir: None,
                project_doc_max_bytes: 0,
            },
            cache_class: "provider-default".to_string(),
            network_policy: "disabled_except_loopback".to_string(),
            tool_policy: "codex_shell_only".to_string(),
            bootstrap_resamples: 1_000,
            limitations: Vec::new(),
        }
    }

    fn test_normalized_usage() -> crate::NormalizedAgentTokenUsage {
        crate::NormalizedAgentTokenUsage {
            contract: crate::AGENT_TOKEN_USAGE_CONTRACT.to_string(),
            run_id: "test".to_string(),
            workload_id: "GD-01".to_string(),
            mode: AgentTokenMode::GitLinearSingleSession,
            accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            model_provider: "openai".to_string(),
            model_id: "test".to_string(),
            model_revision: "test".to_string(),
            reasoning_effort: "medium".to_string(),
            input_tokens: 10,
            cached_input_tokens: None,
            cache_write_input_tokens: None,
            output_tokens: 2,
            reasoning_tokens: None,
            provider_total_tokens: 12,
            completed_turns: 1,
            usage_provenance: "test".to_string(),
        }
    }

    fn pair_test_summary(
        entry: &AgentTokenScheduleEntry,
        infrastructure_failure: Option<&str>,
        valid_attempt: bool,
        accepted_equivalent: bool,
    ) -> AgentTokenRunSummary {
        AgentTokenRunSummary {
            contract: AGENT_TOKEN_RUN_SUMMARY_CONTRACT.to_string(),
            campaign_id: "pair-test".to_string(),
            run_id: entry.run_id.clone(),
            workload_id: entry.workload_id.clone(),
            mode: entry.mode,
            accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            attempt: entry.attempt,
            block_index: entry.block_index,
            randomized_order: entry.randomized_order,
            initial_content_digest: "sha256:initial".to_string(),
            final_content_digest: Some("sha256:final".to_string()),
            codex_exit_code: Some(0),
            codex_timed_out: false,
            elapsed_ms: 1,
            infrastructure_failure: infrastructure_failure.map(str::to_string),
            provider_refusal: false,
            provider_stop_reason: None,
            usage: None,
            transcript: AgentTokenCommandTranscript {
                contract: crate::AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
                run_id: entry.run_id.clone(),
                mode: entry.mode,
                accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
                command_count: 0,
                commands: Vec::new(),
                valid: valid_attempt,
                errors: Vec::new(),
                observed_required_commands: Vec::new(),
            },
            secondary_metrics: crate::AgentTokenSecondaryMetrics::default(),
            evaluator_exit_code: Some(0),
            evaluator_score: Some(100),
            evaluator_accepted: accepted_equivalent,
            browser: AgentTokenBrowserReport {
                contract: crate::AGENT_TOKEN_BROWSER_REPORT_CONTRACT.to_string(),
                workload_id: entry.workload_id.clone(),
                required_for_equivalent_completion: true,
                status: "passed".to_string(),
                desktop_passed: Some(true),
                mobile_passed: Some(true),
                console_errors: Some(0),
                failed_requests: Some(0),
                horizontal_overflow: Some(false),
                notes: Vec::new(),
            },
            workflow_closed: true,
            valid_attempt,
            accepted_equivalent,
            invalid_reasons: Vec::new(),
            failure_reasons: Vec::new(),
        }
    }

    #[test]
    fn resume_fixture_override_must_match_frozen_campaign_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let campaign_dir = temp.path().join("campaign");
        std::fs::create_dir(&campaign_dir).unwrap();
        let frozen = b"{\"fixture\":\"frozen\"}\n";
        std::fs::write(campaign_dir.join("fixture-manifest.json"), frozen).unwrap();
        let identical = temp.path().join("identical.json");
        std::fs::write(&identical, frozen).unwrap();
        let resolved = require_resume_fixture_manifest_identity(&campaign_dir, &identical)
            .expect("byte-identical fixture override");
        assert_eq!(resolved, std::fs::canonicalize(&identical).unwrap());

        let different = temp.path().join("different.json");
        std::fs::write(&different, b"{\"fixture\":\"different\"}\n").unwrap();
        let error = require_resume_fixture_manifest_identity(&campaign_dir, &different)
            .expect_err("different fixture override must fail closed");
        assert!(error.contains("differs from frozen campaign fixture bytes"));
        assert!(error.contains("sha256:"));

        let missing = temp.path().join("missing.json");
        let error = require_resume_fixture_manifest_identity(&campaign_dir, &missing)
            .expect_err("missing fixture override must fail closed");
        assert!(error.contains("Failed to resolve resume fixture manifest"));
        assert!(!campaign_dir.join("resumptions").exists());
    }

    #[test]
    fn report_discloses_served_model_mix_and_fallback_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = test_manifest();
        manifest.runtime.executor = crate::agent_token::AgentTokenExecutor::Claude;
        manifest.model.provider = "anthropic".to_string();
        manifest.model.model_id = "claude-fable-5".to_string();
        manifest.model.model_revision = "sample".to_string();
        manifest.model.reasoning_effort = "max".to_string();
        manifest.claude_model_admission =
            crate::agent_token::AgentTokenClaudeModelAdmission::AsShipped;
        let schedule = build_agent_token_schedule(&manifest);
        let entry = &schedule.entries[0];
        let run_dir = temp.path().join("run");
        std::fs::create_dir_all(run_dir.join("private")).unwrap();
        let raw = run_dir.join("private/codex-events.raw.jsonl");
        std::fs::write(
            &raw,
            concat!(
                "{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-fable-5\",\"tools\":[\"Bash\",\"Read\",\"Grep\",\"Glob\",\"Edit\",\"Write\"],\"mcp_servers\":[]}\n",
                "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-fable-5\",\"content\":[]}}\n",
                "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-opus-5\",\"content\":[{\"type\":\"fallback\"}]}}\n",
                "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"terminal_reason\":\"completed\",\"stop_reason\":\"end_turn\",\"num_turns\":1,\"usage\":{\"input_tokens\":30,\"cache_read_input_tokens\":700,\"cache_creation_input_tokens\":40,\"output_tokens\":9},\"modelUsage\":{\"claude-fable-5\":{\"inputTokens\":20,\"cacheReadInputTokens\":500,\"cacheCreationInputTokens\":30,\"outputTokens\":6,\"canonicalModel\":\"claude-fable-5\"},\"claude-opus-5\":{\"inputTokens\":10,\"cacheReadInputTokens\":200,\"cacheCreationInputTokens\":10,\"outputTokens\":3,\"canonicalModel\":\"claude-opus-5\"}}}\n"
            ),
        )
        .unwrap();
        let imported = crate::agent_token::import_claude_usage_with_outcome(
            &raw,
            &entry.run_id,
            &entry.workload_id,
            entry.mode,
            manifest.accounting_profile,
            &manifest.model,
            manifest.claude_model_admission,
        )
        .unwrap();
        let mut run = pair_test_summary(entry, None, true, true);
        run.campaign_id = manifest.campaign_id.clone();
        run.usage = Some(imported.usage);
        run.provider_stop_reason = Some("end_turn".to_string());
        let runs = vec![run];
        let mut report = build_agent_token_report(&manifest, &schedule, &runs).unwrap();
        let paths = BTreeMap::from([(entry.run_id.clone(), run_dir.join("run-summary.json"))]);

        enrich_agent_token_report_model_composition(&manifest, &runs, &paths, &mut report).unwrap();

        assert_eq!(report.mixed_model_run_count, 1);
        assert_eq!(report.fallback_observed_run_count, 1);
        assert_eq!(report.served_models.len(), 2);
        assert_eq!(report.served_models[0].provider_total_tokens, 556);
        assert_eq!(report.served_models[1].provider_total_tokens, 223);
        let markdown = render_agent_token_report_markdown(&report);
        assert!(markdown.contains("## Served Model Composition"));
        assert!(markdown.contains("Mixed-model runs: `1`"));
        assert!(markdown.contains("`claude-opus-5`"));
    }

    #[test]
    fn recovery_exemption_names_only_the_trailing_contaminated_pair() {
        // A provider-truncated lane keeps a summary that fails per-run checks.
        // The authorized recovery must reach it, so exactly the two lanes of
        // the trailing pair are exempted and no earlier lane is.
        let mut manifest = test_manifest();
        manifest.workload_ids = vec!["GD-01".to_string(), "GD-02".to_string()];
        let schedule = build_agent_token_schedule(&manifest);
        assert_eq!(schedule.entries.len(), 4);
        let runs = schedule
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                // Realistic truncation: one lane of the trailing pair carries
                // the recognized provider limit, its counterpart stays valid,
                // and the first pair is untouched.
                let truncated = index == 2;
                let failure = truncated.then_some("provider_usage_limit");
                pair_test_summary(entry, failure, !truncated, !truncated)
            })
            .collect::<Vec<_>>();

        let exempt = infrastructure_recovery_exempt_run_ids_for(&schedule, runs);

        assert_eq!(exempt.len(), 2, "exemption must cover exactly one pair");
        for entry in &schedule.entries[2..4] {
            assert!(
                exempt.contains(&entry.run_id),
                "trailing lane {} must be exempt",
                entry.run_id
            );
        }
        for entry in &schedule.entries[..2] {
            assert!(
                !exempt.contains(&entry.run_id),
                "earlier lane {} must stay validated",
                entry.run_id
            );
        }
    }

    #[test]
    fn recovery_exemption_is_empty_when_an_earlier_lane_is_invalid() {
        // classify refuses a prefix whose invalid lane precedes the trailing
        // pair, so nothing is exempted and revalidation still rejects it.
        let mut manifest = test_manifest();
        manifest.workload_ids = vec!["GD-01".to_string(), "GD-02".to_string()];
        let schedule = build_agent_token_schedule(&manifest);
        let runs = schedule
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let clean = index != 0;
                pair_test_summary(entry, None, clean, clean)
            })
            .collect::<Vec<_>>();

        assert!(infrastructure_recovery_exempt_run_ids_for(&schedule, runs).is_empty());
    }

    #[test]
    fn resume_accepts_only_an_exact_complete_pair_schedule_prefix() {
        let mut manifest = test_manifest();
        manifest.workload_ids = vec!["GD-01".to_string(), "GD-02".to_string()];
        let schedule = build_agent_token_schedule(&manifest);
        assert_eq!(schedule.entries.len(), 4);
        let summaries = schedule
            .entries
            .iter()
            .map(|entry| pair_test_summary(entry, None, true, true))
            .collect::<Vec<_>>();

        let prefix = exact_schedule_prefix(&schedule, summaries[..2].to_vec()).unwrap();
        assert_eq!(
            prefix
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            schedule.entries[..2]
                .iter()
                .map(|entry| entry.run_id.as_str())
                .collect::<Vec<_>>()
        );

        let partial_error = exact_schedule_prefix(&schedule, summaries[..1].to_vec()).unwrap_err();
        assert!(partial_error.contains("partial pair"), "{partial_error}");

        let hole_error =
            exact_schedule_prefix(&schedule, vec![summaries[0].clone(), summaries[2].clone()])
                .unwrap_err();
        assert!(
            hole_error.contains("after a missing schedule entry"),
            "{hole_error}"
        );

        let mut unexpected = summaries[0].clone();
        unexpected.run_id = "unexpected-run".to_string();
        let unexpected_error = exact_schedule_prefix(&schedule, vec![unexpected]).unwrap_err();
        assert!(
            unexpected_error.contains("absent from the frozen schedule"),
            "{unexpected_error}"
        );
    }

    #[test]
    fn managed_complete_200_accepts_five_pairs_as_its_exact_ten_session_prefix() {
        let mut manifest = test_manifest();
        manifest.campaign_scope = crate::AgentTokenCampaignScope::Complete;
        manifest.attempts_per_cell = 20;
        manifest.workload_ids = ["GD-01", "GD-02", "GD-03", "GD-04", "GD-05"]
            .into_iter()
            .map(str::to_string)
            .collect();
        manifest.ait_edit_root_mode = crate::agent_token::AgentTokenAitEditRootMode::Returned;
        manifest.git_worktree_mode = AgentTokenGitWorktreeMode::CodexAppEquivalentManaged;
        let schedule = build_agent_token_schedule(&manifest);
        assert_eq!(schedule.entries.len(), 200);
        let first_five_pairs = schedule.entries[..10]
            .iter()
            .map(|entry| pair_test_summary(entry, None, true, true))
            .collect::<Vec<_>>();

        let prefix = exact_schedule_prefix(&schedule, first_five_pairs).unwrap();
        assert_eq!(prefix.len(), 10);
        assert_eq!(prefix.last().unwrap().run_id, schedule.entries[9].run_id);
        assert_ne!(schedule.entries[prefix.len()].run_id, prefix[9].run_id);
        assert_eq!(schedule.entries.len() - prefix.len(), 190);
    }

    #[test]
    fn recovered_spawn_partial_is_exact_and_requires_its_missing_counterpart() {
        let mut manifest = test_manifest();
        manifest.campaign_id = AGENT_TOKEN_RECOVERED_SPAWN_CAMPAIGN_ID.to_string();
        manifest.protocol_revision =
            crate::AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION.to_string();
        manifest.campaign_scope = crate::AgentTokenCampaignScope::Complete;
        manifest.ait_sprint_mode = AgentTokenAitSprintMode::On;
        manifest.seed = 2_026_082_832;
        manifest.attempts_per_cell = 20;
        manifest.workload_ids = ["GD-01", "GD-02", "GD-03", "GD-04", "GD-05"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let schedule = build_agent_token_schedule(&manifest);
        assert_eq!(
            schedule.entries[AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX].run_id,
            AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID
        );
        let mut effective_runs = schedule.entries[..=AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX]
            .iter()
            .map(|entry| pair_test_summary(entry, None, true, true))
            .collect::<Vec<_>>();
        let last = effective_runs.last_mut().unwrap();
        last.campaign_id = manifest.campaign_id.clone();
        last.usage = Some(test_normalized_usage());
        last.transcript.valid = true;
        last.transcript.command_count = 1;
        last.transcript.commands = vec!["pwd".to_string()];

        let temp = tempfile::tempdir().unwrap();
        let adjudication = temp
            .path()
            .join("adjudications")
            .join(format!("{AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID}.json"));
        fs::create_dir_all(adjudication.parent().unwrap()).unwrap();
        fs::write(&adjudication, "{}\n").unwrap();
        assert!(exact_recovered_spawn_partial(
            &manifest,
            &schedule,
            temp.path(),
            &effective_runs,
        ));

        let counterpart = &schedule.entries[AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX + 1];
        fs::create_dir_all(temp.path().join("runs").join(&counterpart.run_id)).unwrap();
        assert!(!exact_recovered_spawn_partial(
            &manifest,
            &schedule,
            temp.path(),
            &effective_runs,
        ));
    }

    #[test]
    fn infrastructure_recovery_classifies_only_the_final_contaminated_whole_pair() {
        let mut manifest = test_manifest();
        manifest.workload_ids = vec!["GD-01".to_string(), "GD-02".to_string()];
        let schedule = build_agent_token_schedule(&manifest);
        let mut runs = schedule.entries[..2]
            .iter()
            .map(|entry| pair_test_summary(entry, None, true, true))
            .collect::<Vec<_>>();
        runs.push(pair_test_summary(
            &schedule.entries[2],
            Some("codex_tool_process_spawn_failure"),
            false,
            false,
        ));

        let (ordered, pair_start) =
            classify_infrastructure_recovery_prefix(&schedule, runs).unwrap();
        assert_eq!(pair_start, 2);
        assert_eq!(ordered.len(), 3);

        let mut functional_failure = schedule.entries[..3]
            .iter()
            .map(|entry| pair_test_summary(entry, None, true, true))
            .collect::<Vec<_>>();
        functional_failure[2].accepted_equivalent = false;
        assert!(
            classify_infrastructure_recovery_prefix(&schedule, functional_failure)
                .unwrap_err()
                .contains("lacks a recognized executor infrastructure failure")
        );

        let mut contaminated_prefix = pair_test_summary(
            &schedule.entries[0],
            Some("provider_transport_failure"),
            false,
            false,
        );
        contaminated_prefix
            .invalid_reasons
            .push("infrastructure".to_string());
        let later_pair = schedule.entries[2..]
            .iter()
            .map(|entry| pair_test_summary(entry, None, true, true));
        let error = classify_infrastructure_recovery_prefix(
            &schedule,
            std::iter::once(contaminated_prefix)
                .chain(std::iter::once(pair_test_summary(
                    &schedule.entries[1],
                    None,
                    true,
                    true,
                )))
                .chain(later_pair)
                .collect(),
        )
        .unwrap_err();
        assert!(error.contains("precedes the recoverable pair"), "{error}");
    }

    #[test]
    fn successor_resume_admits_valid_unaccepted_protocol_27_prefix_without_retry() {
        let manifest = test_manifest();
        let schedule = build_agent_token_schedule(&manifest);
        let mut summaries = schedule
            .entries
            .iter()
            .map(|entry| pair_test_summary(entry, None, true, true))
            .collect::<Vec<_>>();
        summaries[0].accepted_equivalent = false;

        assert!(validate_resume_protocol_revision(
            AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION
        )
        .is_ok());
        assert_eq!(
            validate_resume_prefix_outcomes(
                AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION,
                &summaries
            ),
            Ok(true)
        );

        summaries[0].valid_attempt = false;
        let error = validate_resume_prefix_outcomes(
            AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION,
            &summaries,
        )
        .unwrap_err();
        assert!(error.contains("unadjudicated invalid run"), "{error}");
    }

    #[test]
    fn complete_predecessors_other_than_protocol_27_remain_read_only() {
        assert!(validate_resume_protocol_revision(AGENT_TOKEN_PROTOCOL_REVISION).is_ok());
        assert!(validate_resume_protocol_revision(
            AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION
        )
        .is_ok());
        assert!(validate_resume_protocol_revision(
            crate::AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION
        )
        .is_ok());
        let error = validate_resume_protocol_revision(
            crate::AGENT_TOKEN_200_SESSION_PREDECESSOR_PROTOCOL_REVISION,
        )
        .unwrap_err();
        assert!(error.contains("read-only"), "{error}");
    }

    #[test]
    fn protocol_46_alone_admits_its_absent_legacy_execution_workspace() {
        assert!(run_manifest_execution_workspace_matches(
            AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION,
            "",
            "workspace",
        ));
        assert!(run_manifest_execution_workspace_matches(
            AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION,
            "workspace",
            "workspace",
        ));
        assert!(!run_manifest_execution_workspace_matches(
            AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION,
            "different-workspace",
            "workspace",
        ));
        assert!(!run_manifest_execution_workspace_matches(
            AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION,
            "",
            "git-worktree-runtime/git-task-worktree",
        ));
        assert!(!run_manifest_execution_workspace_matches(
            AGENT_TOKEN_PROTOCOL_REVISION,
            "",
            "workspace",
        ));
        assert!(!run_manifest_execution_workspace_matches(
            crate::AGENT_TOKEN_MANAGED_WORKTREE_PREFLIGHT_PREDECESSOR_PROTOCOL_REVISION,
            "",
            "workspace",
        ));
    }

    #[test]
    fn protocols_27_30_and_current_retain_proof_backed_implicit_main_admission() {
        assert!(protocol_requires_git_start_state_proof(
            AGENT_TOKEN_PROTOCOL_REVISION
        ));
        assert!(protocol_requires_git_start_state_proof(
            AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION
        ));
        assert!(protocol_requires_git_start_state_proof(
            crate::AGENT_TOKEN_PROMPTED_INSPECTION_PREDECESSOR_PROTOCOL_REVISION
        ));
        assert!(protocol_requires_git_start_state_proof(
            AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION
        ));
        assert!(!protocol_requires_git_start_state_proof(
            crate::AGENT_TOKEN_200_SESSION_PREDECESSOR_PROTOCOL_REVISION
        ));
    }

    #[test]
    fn resume_provenance_hashes_the_exact_executing_binary() {
        let (runner_program, runner_sha256) = current_runner_provenance().unwrap();
        assert!(runner_program.is_file());
        assert_eq!(
            runner_sha256,
            sha256_digest(&fs::read(runner_program).unwrap())
        );
        assert_eq!(
            AGENT_TOKEN_CAMPAIGN_RESUME_CONTRACT,
            "ait-agent-token-campaign-resume/v2"
        );
    }

    #[test]
    fn project_document_loading_labels_default_and_diagnostic_modes_exactly() {
        assert_eq!(
            project_document_loading_label(0),
            "disabled_symmetrically_project_doc_max_bytes_0"
        );
        assert_eq!(
            project_document_loading_label(8_192),
            "enabled_symmetrically_pilot_diagnostic_project_doc_max_bytes_8192"
        );
    }

    #[test]
    fn measured_codex_command_uses_and_records_default_feature_settings() {
        let workspace = tempfile::tempdir().unwrap();
        let metadata = workspace.path().join("git-metadata");
        let container = workspace.path().join("git-worktree-container");
        let task = container.join("git-task-worktree");
        let add_dirs = [metadata.clone(), container];
        let git_write_exceptions = [workspace.path().join(".git"), metadata, task.join(".git")];
        let command = build_codex_command(
            &test_manifest(),
            workspace.path(),
            &add_dirs,
            &git_write_exceptions,
        )
        .unwrap();
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(args.first().map(String::as_str), Some("exec"));
        assert!(args.iter().any(|arg| arg == "test-model"));
        assert!(!args.iter().any(|arg| arg == "--sandbox"));
        assert!(!args.iter().any(|arg| arg == "workspace-write"));
        assert!(!args.iter().any(|arg| arg == "danger-full-access"));
        assert!(args
            .iter()
            .any(|arg| arg == "default_permissions=\"ait_benchmark_local_v1\""));
        let profile_arg = args
            .iter()
            .find(|arg| arg.starts_with("permissions.ait_benchmark_local_v1="))
            .expect("permission-profile override");
        assert!(profile_arg.contains("extends=\":workspace\""));
        assert!(profile_arg.contains("network={ enabled=false }"));
        assert!(profile_arg.contains("git-metadata"));
        assert!(profile_arg.contains("git-worktree-container"));
        assert!(profile_arg.contains("git-task-worktree/.git"));
        assert!(args.iter().any(|arg| arg == "project_doc_max_bytes=0"));
        assert!(!args.iter().any(|arg| arg == "--add-dir"));
        assert!(!args
            .iter()
            .any(|arg| arg == "--enable" || arg == "--disable"));
        assert!(codex_enabled_feature_overrides().is_empty());
        assert!(codex_disabled_feature_overrides().is_empty());
        assert!(!command
            .get_envs()
            .any(|(name, _)| { matches!(name.to_str(), Some("GIT_DIR" | "GIT_WORK_TREE")) }));
    }

    #[test]
    fn permission_profile_rejects_git_write_exception_outside_declared_roots() {
        let workspace = tempfile::tempdir().unwrap();
        let unrelated = tempfile::tempdir().unwrap();
        let error =
            build_codex_permission_profile(workspace.path(), &[], &[unrelated.path().join(".git")])
                .unwrap_err();
        assert!(error.contains("outside the declared workspaces"));
    }

    #[test]
    fn app_managed_permission_profile_excludes_the_parent_main_git_pointer() {
        let temp = tempfile::tempdir().unwrap();
        let main_workspace = temp.path().join("workspace");
        let metadata = temp.path().join("git-metadata");
        let container = temp.path().join("git-worktree-runtime");
        let task_worktree = container.join("git-task-worktree");
        let exceptions = git_write_exceptions_for_mode(
            AgentTokenGitWorktreeMode::CodexAppEquivalentManaged,
            &main_workspace,
            &metadata,
            &task_worktree,
        );

        assert!(!exceptions.contains(&main_workspace.join(".git")));
        assert!(exceptions.contains(&metadata));
        assert!(exceptions.contains(&task_worktree.join(".git")));
        let profile = build_codex_permission_profile(
            &task_worktree,
            &[metadata.clone(), container],
            &exceptions,
        )
        .unwrap();
        assert!(codex_run_permission_profile_matches(
            &profile,
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenGitWorktreeMode::CodexAppEquivalentManaged,
        ));
        assert!(!profile
            .git_write_exceptions
            .contains(&main_workspace.join(".git").display().to_string()));

        let mut overbroad = profile;
        overbroad
            .git_write_exceptions
            .push(main_workspace.join(".git").display().to_string());
        assert!(!codex_run_permission_profile_matches(
            &overbroad,
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenGitWorktreeMode::CodexAppEquivalentManaged,
        ));
    }

    #[test]
    fn evidence_validation_resolves_relative_campaign_roots() {
        let resolved = resolve_campaign_evidence_dir(Path::new(".")).unwrap();
        assert!(resolved.is_absolute());
        assert_eq!(resolved, std::fs::canonicalize(".").unwrap());
    }

    #[test]
    fn executor_preflight_accepts_exactly_thirty_successful_read_only_commands() {
        let temp = tempfile::tempdir().unwrap();
        let events = temp.path().join("events.jsonl");
        let mut values = (1..=AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT)
            .flat_map(|index| {
                [
                    serde_json::json!({
                        "type": "item.started",
                        "item": {
                            "id": format!("item_{index}"),
                            "type": "command_execution",
                            "command": "/bin/zsh -lc /bin/pwd",
                            "aggregated_output": "",
                            "exit_code": null,
                            "status": "in_progress"
                        }
                    }),
                    serde_json::json!({
                        "type": "item.completed",
                        "item": {
                            "id": format!("item_{index}"),
                            "type": "command_execution",
                            "command": "/bin/zsh -lc /bin/pwd",
                            "aggregated_output": "/tmp/preflight\n",
                            "exit_code": 0,
                            "status": "completed"
                        }
                    }),
                ]
            })
            .collect::<Vec<_>>();
        values.push(serde_json::json!({
            "type": "turn.completed",
            "usage": {
                "input_tokens": 100,
                "cached_input_tokens": 50,
                "cache_write_input_tokens": 0,
                "output_tokens": 10,
                "reasoning_output_tokens": 2
            }
        }));
        write_json_lines_new(&events, &values).unwrap();
        let observation = inspect_executor_preflight_events(&events);
        let manifest = test_manifest();
        let usage = import_codex_usage(
            &events,
            "preflight",
            "executor-preflight",
            AgentTokenMode::GitLinearSingleSession,
            manifest.accounting_profile,
            &manifest.model,
        )
        .unwrap();
        let process = TimedProcessResult {
            exit_code: Some(0),
            timed_out: false,
            elapsed_ms: 1,
        };
        let reasons = executor_preflight_failure_reasons(
            &observation,
            &process,
            "sha256:same",
            Some("sha256:same"),
            None,
            Some(&usage),
            Vec::new(),
        );

        assert!(reasons.is_empty(), "{reasons:?}");
        assert_eq!(observation.started_command_count, 30);
        assert_eq!(observation.observed_command_count, 30);
        assert_eq!(observation.distinct_command_count, 30);
        assert_eq!(observation.successful_command_count, 30);
        assert_eq!(observation.sequential_violation_count, 0);
        assert_eq!(observation.unexpected_tool_item_count, 0);
    }

    #[test]
    fn executor_preflight_exact_command_rejects_embedded_or_extended_pwd_text() {
        for accepted in [
            "/bin/pwd",
            "/bin/zsh -c /bin/pwd",
            "/bin/zsh -lc /bin/pwd",
            "/bin/zsh -lc '/bin/pwd'",
            "/bin/zsh -lc \"/bin/pwd\"",
            "/bin/bash -c '/bin/pwd'",
            "/bin/sh -lc \"/bin/pwd\"",
        ] {
            assert!(
                is_exact_executor_preflight_command(accepted),
                "expected exact probe wrapper to pass: {accepted}"
            );
        }
        for rejected in [
            "echo /bin/pwd",
            "/bin/pwd -P",
            "/bin/zsh -lc '/bin/pwd && true'",
            "/bin/zsh -lc 'echo /bin/pwd'",
            "/bin/zsh -ic /bin/pwd",
            "/bin/zsh -c -- /bin/pwd",
            "/bin/zsh -c '/bin/pwd' extra",
            "/usr/bin/zsh -c /bin/pwd",
            "/usr/bin/env /bin/pwd",
        ] {
            assert!(
                !is_exact_executor_preflight_command(rejected),
                "expected non-exact probe to fail: {rejected}"
            );
        }
    }

    #[test]
    fn executor_preflight_rejects_overlapping_command_lifecycles() {
        let temp = tempfile::tempdir().unwrap();
        let events = temp.path().join("events.jsonl");
        let values = [
            serde_json::json!({
                "type": "item.started",
                "item": {"id": "item_1", "type": "command_execution", "command": "/bin/pwd"}
            }),
            serde_json::json!({
                "type": "item.started",
                "item": {"id": "item_2", "type": "command_execution", "command": "/bin/pwd"}
            }),
            serde_json::json!({
                "type": "item.completed",
                "item": {"id": "item_1", "type": "command_execution", "command": "/bin/pwd", "exit_code": 0}
            }),
            serde_json::json!({
                "type": "item.completed",
                "item": {"id": "item_2", "type": "command_execution", "command": "/bin/pwd", "exit_code": 0}
            }),
        ];
        write_json_lines_new(&events, &values).unwrap();

        let observation = inspect_executor_preflight_events(&events);

        assert_eq!(observation.started_command_count, 2);
        assert_eq!(observation.observed_command_count, 2);
        assert_eq!(observation.sequential_violation_count, 1);
        assert!(observation.errors.is_empty(), "{:?}", observation.errors);
    }

    #[test]
    fn executor_preflight_rejects_malformed_failed_mutating_and_unhealthy_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let events = temp.path().join("events.jsonl");
        fs::write(
            &events,
            concat!(
                "{not-json}\n",
                "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_1\",",
                "\"type\":\"command_execution\",\"command\":\"/bin/pwd && touch changed\",",
                "\"aggregated_output\":\"\",\"exit_code\":null,\"status\":\"failed\"}}\n",
                "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_2\",",
                "\"type\":\"file_change\",\"changes\":[]}}\n"
            ),
        )
        .unwrap();
        let observation = inspect_executor_preflight_events(&events);
        let process = TimedProcessResult {
            exit_code: None,
            timed_out: true,
            elapsed_ms: 1,
        };
        let reasons = executor_preflight_failure_reasons(
            &observation,
            &process,
            "sha256:before",
            Some("sha256:after"),
            Some("codex_tool_process_spawn_failure"),
            None,
            vec!["captured provider failure".to_string()],
        );

        for expected in [
            "invalid JSON",
            "infrastructure unavailable",
            "timed out",
            "exited with None",
            "observed 1 command items",
            "0 successful and 1 failed",
            "outside the exact /bin/pwd probe",
            "1 file-change items",
            "provider usage is missing",
            "workspace content changed",
        ] {
            assert!(
                reasons.iter().any(|reason| reason.contains(expected)),
                "missing {expected:?} in {reasons:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_executor_preflight_starts_zero_candidate_lanes() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let codex = temp.path().join("fake-codex");
        fs::write(
            &codex,
            concat!(
                "#!/bin/sh\n",
                "if [ \"$1\" = \"--version\" ]; then echo 'fake-codex 1'; exit 0; fi\n",
                "if [ \"$1\" = \"sandbox\" ]; then\n",
                "  shift\n",
                "  probe_cwd=\n",
                "  while [ \"$#\" -gt 0 ]; do\n",
                "    case \"$1\" in\n",
                "      --permission-profile|--config) shift 2 ;;\n",
                "      --cd) probe_cwd=$2; shift 2 ;;\n",
                "      --) shift; break ;;\n",
                "      *) exit 91 ;;\n",
                "    esac\n",
                "  done\n",
                "  cd \"$probe_cwd\" || exit 92\n",
                "  exec \"$@\"\n",
                "fi\n",
                "printf '%s\\n' '{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":10,\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":2,\"reasoning_output_tokens\":0}}'\n"
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&codex).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&codex, permissions).unwrap();
        let fixture = temp.path().join("fixture.json");
        fs::write(&fixture, "{}\n").unwrap();
        let mut manifest = test_manifest();
        manifest.campaign_id = "preflight-stops-before-candidates".to_string();
        manifest.model.model_id = "gpt-test".to_string();
        manifest.model.model_revision = "gpt-test-revision".to_string();
        manifest.runtime.codex_program = codex;
        manifest.runtime.ait_program = PathBuf::from("git");
        manifest.runtime.git_program = PathBuf::from("git");
        manifest.runtime.node_program = PathBuf::from("git");
        manifest.runtime.fixture_manifest = fixture;
        let manifest_path = temp.path().join("campaign.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let output = temp.path().join("evidence");

        let execution = run_agent_token_campaign(&manifest_path, &output, Some(1)).unwrap();

        assert!(!execution.preflight_passed);
        assert!(execution.git_worktree_permission_preflight_passed);
        assert_eq!(execution.scheduled_pair_count, 1);
        assert_eq!(execution.requested_pair_count, 1);
        assert_eq!(execution.executed_run_count, 0);
        assert_eq!(execution.completed_pair_count, 0);
        assert!(execution.stopped_early);
        assert!(execution
            .stop_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("executor_preflight_failed:")));
        assert_eq!(fs::read_dir(output.join("runs")).unwrap().count(), 0);
        assert!(output.join("executor-preflight-report.json").is_file());
        assert!(output
            .join("git-worktree-permission-preflight-report.json")
            .is_file());
        assert!(output.join("aggregate-report.json").is_file());
    }

    #[test]
    fn pair_slice_rejects_zero_and_counts_beyond_the_frozen_schedule() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("fixture.json");
        fs::write(&fixture, "{}\n").unwrap();
        let mut manifest = test_manifest();
        manifest.model.model_id = "gpt-test".to_string();
        manifest.model.model_revision = "gpt-test-revision".to_string();
        manifest.runtime.fixture_manifest = fixture;
        let manifest_path = temp.path().join("campaign.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        assert_eq!(
            run_agent_token_campaign(&manifest_path, &temp.path().join("zero"), Some(0))
                .unwrap_err(),
            "max_pairs must be greater than zero when supplied"
        );
        assert!(
            run_agent_token_campaign(&manifest_path, &temp.path().join("too-many"), Some(2))
                .unwrap_err()
                .contains("exceeds the 1 scheduled pairs")
        );
        assert!(!temp.path().join("zero").exists());
        assert!(!temp.path().join("too-many").exists());
    }

    #[test]
    fn infrastructure_failure_stops_immediately_without_starting_the_pair_counterpart() {
        let mut manifest = test_manifest();
        manifest.workload_ids = vec!["GD-01".to_string(), "GD-02".to_string()];
        let schedule = build_agent_token_schedule(&manifest);
        let calls = std::cell::Cell::new(0);

        let (runs, stop_reason) = execute_agent_token_pairs(&schedule.entries, 2, true, |entry| {
            calls.set(calls.get() + 1);
            Ok(pair_test_summary(
                entry,
                Some("codex_tool_process_spawn_failure"),
                false,
                false,
            ))
        })
        .unwrap();

        assert_eq!(calls.get(), 1);
        assert_eq!(runs.len(), 1);
        assert!(stop_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("codex_tool_process_spawn_failure")));
    }

    #[test]
    fn valid_candidate_defect_is_retained_without_stopping_the_next_pair() {
        let mut manifest = test_manifest();
        manifest.workload_ids = vec!["GD-01".to_string(), "GD-02".to_string()];
        let schedule = build_agent_token_schedule(&manifest);
        let calls = std::cell::Cell::new(0);

        let (runs, stop_reason) = execute_agent_token_pairs(&schedule.entries, 2, true, |entry| {
            let call_index = calls.get();
            calls.set(call_index + 1);
            Ok(pair_test_summary(entry, None, true, call_index != 0))
        })
        .unwrap();

        assert_eq!(calls.get(), 4);
        assert_eq!(runs.len(), 4);
        assert!(runs[0].valid_attempt);
        assert!(!runs[0].accepted_equivalent);
        assert!(stop_reason.is_none());
    }

    #[test]
    fn invalid_attempt_finishes_its_pair_then_stops_before_the_next_pair() {
        let mut manifest = test_manifest();
        manifest.workload_ids = vec!["GD-01".to_string(), "GD-02".to_string()];
        let schedule = build_agent_token_schedule(&manifest);
        let calls = std::cell::Cell::new(0);

        let (runs, stop_reason) = execute_agent_token_pairs(&schedule.entries, 2, true, |entry| {
            let call_index = calls.get();
            calls.set(call_index + 1);
            Ok(pair_test_summary(entry, None, call_index != 0, false))
        })
        .unwrap();

        assert_eq!(calls.get(), 2);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].workload_id, runs[1].workload_id);
        assert_eq!(runs[0].attempt, runs[1].attempt);
        assert!(stop_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("paired_invalid_attempt")));
    }

    #[test]
    fn both_measured_prompts_carry_the_identical_neutral_inspection_admission() {
        // Protocol .31 removed a suggestion from the Git prompt but a
        // permission from the AIT prompt, leaving AIT with a bare closed-set
        // prohibition while Git stayed silently unrestricted. Both prompts must
        // now carry the same neutral sentence and name no inspection command.
        const ADMISSION: &str =
            "Local read-only inspection of this repository is neither required nor prohibited.";
        let manifest = test_manifest();
        let worktree = Path::new("/tmp/git-task-worktree");

        for (mode, sprint, git_worktree_mode) in [
            (
                AgentTokenMode::GitLinearSingleSession,
                None,
                AgentTokenGitWorktreeMode::AgentManaged,
            ),
            (
                AgentTokenMode::GitLinearSingleSession,
                None,
                AgentTokenGitWorktreeMode::CodexAppEquivalentManaged,
            ),
            (
                AgentTokenMode::AitLinearSingleSession,
                Some(AgentTokenAitSprintMode::Off),
                AgentTokenGitWorktreeMode::AgentManaged,
            ),
            (
                AgentTokenMode::AitLinearSingleSession,
                Some(AgentTokenAitSprintMode::On),
                AgentTokenGitWorktreeMode::AgentManaged,
            ),
        ] {
            let mut manifest = manifest.clone();
            manifest.git_worktree_mode = git_worktree_mode;
            if let Some(sprint) = sprint {
                manifest.ait_sprint_mode = sprint;
            }
            let entry = AgentTokenScheduleEntry {
                run_id: "test-b001-gd-01".to_string(),
                workload_id: "GD-01".to_string(),
                mode,
                attempt: 1,
                block_index: 1,
                randomized_order: 1,
            };
            let prompt = build_measured_prompt(
                &manifest,
                &entry,
                "repair the game",
                Some(worktree),
                None,
                Some(Path::new("/benchmark/ait-task-worktree")),
            );

            assert!(
                prompt.matches(ADMISSION).count() == 1,
                "{mode:?}/{sprint:?}/{git_worktree_mode:?} prompt must carry exactly one admission sentence: {prompt}"
            );
            for coaching in [
                "git status",
                "git diff",
                "git log",
                "git show",
                "git rev-parse",
                "ait status",
                "ait diff",
                "ait blame",
            ] {
                assert!(
                    !prompt.to_ascii_lowercase().contains(coaching),
                    "{mode:?}/{sprint:?}/{git_worktree_mode:?} prompt names inspection command {coaching:?}: {prompt}"
                );
            }
        }
    }

    #[test]
    fn measured_prompt_teaches_only_the_admitted_lifecycle_without_inspection_coaching() {
        let manifest = test_manifest();
        let entry = AgentTokenScheduleEntry {
            run_id: "test-b001-gd-01-ait".to_string(),
            workload_id: "GD-01".to_string(),
            mode: AgentTokenMode::AitLinearSingleSession,
            attempt: 1,
            block_index: 1,
            randomized_order: 1,
        };
        let prompt = build_measured_prompt(
            &manifest,
            &entry,
            "repair the game",
            None,
            None,
            Some(Path::new("/benchmark/ait-task-worktree")),
        );
        assert!(prompt.contains("Use the prepared local AIT repository"));
        assert!(prompt.contains("task start --title ... --intent ... --edit-root /benchmark/ait-task-worktree --local --json"));
        assert!(prompt.contains("Retain the returned `task_id`"));
        assert!(prompt.contains("--edit-root /benchmark/ait-task-worktree"));
        assert!(prompt.contains("task finish <returned-task-id> --message ... --local"));
        assert!(prompt.contains("complete AIT lifecycle command set for this run"));
        assert!(prompt.contains("do not invoke any additional AIT lifecycle or management command"));
        assert!(prompt.contains("Do not invoke `git` for any purpose"));
        assert!(prompt.contains("including after project validation"));
        assert!(prompt.contains("This candidate intentionally has no Git repository"));
        assert!(prompt.contains("snapshot create --message ... --json"));
        for inspection_hint in ["ait status", "ait diff", "ait blame"] {
            assert!(
                !prompt.to_ascii_lowercase().contains(inspection_hint),
                "measured AIT prompt contains inspection coaching {inspection_hint:?}: {prompt}"
            );
        }
        for bootstrap in ["first-use", "ait init", "config set", "baseline"] {
            assert!(
                !prompt.to_ascii_lowercase().contains(bootstrap),
                "steady-state AIT prompt contains bootstrap treatment {bootstrap:?}: {prompt}"
            );
        }
        let mut returned_manifest = manifest.clone();
        returned_manifest.ait_edit_root_mode =
            crate::agent_token::AgentTokenAitEditRootMode::Returned;
        let returned_prompt = build_measured_prompt(
            &returned_manifest,
            &entry,
            "repair the game",
            None,
            None,
            None,
        );
        assert!(returned_prompt.contains("enter the returned physical `edit_root`"));
        assert!(returned_prompt.contains("using `next_action.command`"));
        assert!(!returned_prompt.contains("--edit-root"));
        for retired in [
            "ait-server",
            "remote",
            "push",
            "pull",
            "review",
            "land",
            "workflow ready",
            "queue summary",
            "task list",
            "change list",
            "task audit",
        ] {
            assert!(
                !prompt.to_ascii_lowercase().contains(retired),
                "measured AIT prompt contains retired treatment {retired:?}: {prompt}"
            );
        }
        let fixture_root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/agent-token-game-v1/workloads");
        for workload_id in ["GD-01", "GD-02", "GD-03", "GD-04", "GD-05"] {
            let shared_task = fs::read_to_string(
                fixture_root
                    .join(workload_id)
                    .join("overlay")
                    .join("TASK.txt"),
            )
            .unwrap();
            let fixture_entry = AgentTokenScheduleEntry {
                run_id: format!("test-{workload_id}-ait"),
                workload_id: workload_id.to_string(),
                mode: AgentTokenMode::AitLinearSingleSession,
                attempt: 1,
                block_index: 1,
                randomized_order: 1,
            };
            let fixture_prompt = build_measured_prompt(
                &manifest,
                &fixture_entry,
                shared_task.as_str(),
                None,
                None,
                Some(Path::new("/benchmark/ait-task-worktree")),
            );
            for retired in ["ait-server", "remote", "review", "land"] {
                assert!(
                    !fixture_prompt.to_ascii_lowercase().contains(retired),
                    "{workload_id} prompt contains retired treatment {retired:?}: {fixture_prompt}"
                );
            }
        }

        let mut sprint_on_manifest = manifest.clone();
        sprint_on_manifest.ait_sprint_mode = AgentTokenAitSprintMode::On;
        sprint_on_manifest.workload_ids = ["GD-01", "GD-02", "GD-03", "GD-04", "GD-05"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let sprint_prompt = build_measured_prompt(
            &sprint_on_manifest,
            &entry,
            "repair the game",
            None,
            None,
            Some(Path::new("/benchmark/ait-task-worktree")),
        );
        assert!(sprint_prompt.contains("Sprint mode is on"));
        assert!(sprint_prompt.contains("docs/sprints/benchmark_task.md"));
        assert!(
            sprint_prompt.contains("[plan-ref: agent-token-benchmark/test-b001-gd-01-ait/root]")
        );
        assert!(sprint_prompt.contains(
            "task start --from docs/sprints/benchmark_task.md#agent-token-benchmark/test-b001-gd-01-ait/implement"
        ));
        assert!(sprint_prompt.contains("do not run a separate plan sync"));
        assert!(sprint_prompt.contains("automatically close the exact sprint checklist item"));
        assert!(!sprint_prompt.contains("task start --title"));

        let mut first_use_manifest = manifest.clone();
        first_use_manifest.accounting_profile =
            crate::AgentTokenAccountingProfile::FirstUseTotalCost;
        let first_use_prompt = build_measured_prompt(
            &first_use_manifest,
            &entry,
            "repair the game",
            None,
            None,
            Some(Path::new("/benchmark/ait-task-worktree")),
        );
        assert!(first_use_prompt.contains("ait init"));
        assert!(first_use_prompt.contains("--workflow-mode solo_local --sprint off"));
        assert!(first_use_prompt.contains("snapshot create --message ... --json"));
        assert!(first_use_prompt.contains("Do not invoke `git` for any purpose"));
        assert!(first_use_prompt.contains("including after project validation"));
        assert!(first_use_prompt.contains("This candidate intentionally has no Git repository"));
        for inspection_hint in ["ait status", "ait diff", "ait blame"] {
            assert!(
                !first_use_prompt
                    .to_ascii_lowercase()
                    .contains(inspection_hint),
                "first-use AIT prompt contains inspection coaching {inspection_hint:?}: {first_use_prompt}"
            );
        }
        for retired in ["ait-server", "remote", "review", "land"] {
            assert!(
                !first_use_prompt.to_ascii_lowercase().contains(retired),
                "first-use AIT prompt contains retired treatment {retired:?}: {first_use_prompt}"
            );
        }

        let git_entry = AgentTokenScheduleEntry {
            mode: AgentTokenMode::GitLinearSingleSession,
            ..entry
        };
        let git_worktree = Path::new("/benchmark/git-task-worktree");
        let git_metadata = Path::new("/benchmark/git-metadata");
        let git_prompt = build_measured_prompt(
            &manifest,
            &git_entry,
            "repair the game",
            Some(git_worktree),
            Some(git_metadata),
            Some(Path::new("/benchmark/ait-task-worktree")),
        );
        for bootstrap in ["first-use", "git init", "git config", "baseline"] {
            assert!(
                !git_prompt.to_ascii_lowercase().contains(bootstrap),
                "steady-state Git prompt contains bootstrap treatment {bootstrap:?}: {git_prompt}"
            );
        }
        for required in [
            "worktree add -b benchmark-task /benchmark/git-task-worktree main",
            "equivalent `git worktree add -b benchmark-task /benchmark/git-task-worktree`",
            "runner has proven that the current `HEAD` is this clean `main`",
            "merge --ff-only benchmark-task",
            "worktree remove /benchmark/git-task-worktree",
            "branch -d benchmark-task",
            "exactly one candidate commit",
        ] {
            assert!(
                git_prompt.contains(required),
                "Git prompt is missing {required:?}: {git_prompt}"
            );
        }
        assert!(git_prompt.contains("Do not copy or redirect `.git`"));
        assert!(git_prompt.contains("set `GIT_DIR` or `GIT_WORK_TREE`"));
        assert!(git_prompt.contains("do not invoke `ait`"));
        for inspection_hint in ["git status", "git diff", "git log", "git show", "git blame"] {
            assert!(
                !git_prompt.to_ascii_lowercase().contains(inspection_hint),
                "measured Git prompt contains inspection coaching {inspection_hint:?}: {git_prompt}"
            );
        }
        let mut managed_manifest = manifest.clone();
        managed_manifest.ait_edit_root_mode =
            crate::agent_token::AgentTokenAitEditRootMode::Returned;
        managed_manifest.git_worktree_mode = AgentTokenGitWorktreeMode::CodexAppEquivalentManaged;
        let managed_prompt = build_measured_prompt(
            &managed_manifest,
            &git_entry,
            "repair the game",
            Some(git_worktree),
            Some(git_metadata),
            Some(Path::new("/benchmark/ait-task-worktree")),
        );
        for required in [
            "already begin inside the detached managed worktree",
            "created by the benchmark host from its proven clean `main` HEAD before this model session",
            "Leave the resulting changes in place for host closeout",
            "Do not leave the current worktree or access the parent main worktree",
            "after the terminal model event",
            "outside model-token accounting",
        ] {
            assert!(
                managed_prompt.contains(required),
                "managed Git prompt is missing {required:?}: {managed_prompt}"
            );
        }
        for model_owned in [
            "worktree add -b benchmark-task",
            "merge --ff-only benchmark-task",
            "branch -d benchmark-task",
            "exactly one candidate commit there",
        ] {
            assert!(
                !managed_prompt.contains(model_owned),
                "managed Git prompt requires host-owned step {model_owned:?}: {managed_prompt}"
            );
        }
        let first_use_git_prompt = build_measured_prompt(
            &first_use_manifest,
            &git_entry,
            "repair the game",
            Some(git_worktree),
            Some(git_metadata),
            Some(Path::new("/benchmark/ait-task-worktree")),
        );
        assert!(first_use_git_prompt
            .contains("init --initial-branch=main --separate-git-dir /benchmark/git-metadata ."));
        assert!(first_use_git_prompt.contains("`user.name` to `AIT Benchmark Agent`"));
        assert!(first_use_git_prompt.contains("`user.email` to `benchmark-agent@example.invalid`"));
        assert!(first_use_git_prompt.contains("exactly one baseline commit before editing"));
        assert!(first_use_git_prompt
            .contains("worktree add -b benchmark-task /benchmark/git-task-worktree main"));
        assert!(!first_use_git_prompt.contains("or the equivalent"));
        assert!(first_use_git_prompt.contains("do not invoke `ait`"));
        for inspection_hint in ["git status", "git diff", "git log", "git show", "git blame"] {
            assert!(
                !first_use_git_prompt
                    .to_ascii_lowercase()
                    .contains(inspection_hint),
                "first-use Git prompt contains inspection coaching {inspection_hint:?}: {first_use_git_prompt}"
            );
        }
    }

    #[test]
    fn git_start_state_proof_requires_clean_symbolic_main_matching_head() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let metadata = temp.path().join("git-metadata");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("game.txt"), "baseline\n").unwrap();
        prepare_empty_directory(&metadata, "Git metadata").unwrap();
        let manifest = test_manifest();
        let mut events = Vec::new();
        let mut sequence = 1;
        bootstrap_git(&manifest, &workspace, &metadata, &mut events, &mut sequence).unwrap();

        let proof = capture_git_start_state_proof(&manifest, "clean-main", &workspace);
        assert!(proof.passed, "{:?}", proof.failure_reasons);
        assert_eq!(proof.current_branch.as_deref(), Some("main"));
        assert_eq!(proof.head_oid, proof.main_oid);
        assert_eq!(proof.status_porcelain.as_deref(), Some(""));

        fs::write(workspace.join("dirty.txt"), "dirty\n").unwrap();
        let dirty = capture_git_start_state_proof(&manifest, "dirty-main", &workspace);
        assert!(!dirty.passed);
        assert!(!dirty.clean);
        fs::remove_file(workspace.join("dirty.txt")).unwrap();

        run_checked_event(
            &manifest.runtime.git_program,
            &["switch", "-c", "other"],
            &workspace,
            "test",
            &mut events,
            &mut sequence,
        )
        .unwrap();
        let other = capture_git_start_state_proof(&manifest, "other-branch", &workspace);
        assert!(!other.passed);
        assert_eq!(other.current_branch.as_deref(), Some("other"));

        run_checked_event(
            &manifest.runtime.git_program,
            &["switch", "--detach", "main"],
            &workspace,
            "test",
            &mut events,
            &mut sequence,
        )
        .unwrap();
        run_checked_event(
            &manifest.runtime.git_program,
            &["commit", "--allow-empty", "-m", "Detached head"],
            &workspace,
            "test",
            &mut events,
            &mut sequence,
        )
        .unwrap();
        let detached = capture_git_start_state_proof(&manifest, "detached", &workspace);
        assert!(!detached.passed);
        assert_eq!(detached.current_branch, None);
        assert!(!detached.head_matches_main);
    }

    #[test]
    fn git_bootstrap_linked_worktree_and_closeout_converge_on_clean_main() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let worktree_container = temp.path().join("git-worktree-container");
        let task_worktree = worktree_container.join("task");
        let metadata = temp.path().join("git-metadata");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&worktree_container).unwrap();
        fs::write(workspace.join("game.txt"), "baseline\n").unwrap();
        prepare_empty_directory(&metadata, "Git metadata").unwrap();

        let manifest = test_manifest();
        let mut events = Vec::new();
        let mut sequence = 1;
        bootstrap_git(&manifest, &workspace, &metadata, &mut events, &mut sequence).unwrap();
        assert!(workspace.join(".git").is_file());
        assert!(metadata.join("HEAD").is_file());
        let start_proof = capture_git_start_state_proof(&manifest, "closeout", &workspace);
        assert!(start_proof.passed, "{:?}", start_proof.failure_reasons);

        run_checked_event(
            &manifest.runtime.git_program,
            &[
                "worktree",
                "add",
                "-b",
                "benchmark-task",
                task_worktree.to_str().unwrap(),
            ],
            &workspace,
            "candidate",
            &mut events,
            &mut sequence,
        )
        .unwrap();
        fs::write(task_worktree.join("game.txt"), "repaired\n").unwrap();
        run_checked_event(
            &manifest.runtime.git_program,
            &["add", "--all"],
            &task_worktree,
            "candidate",
            &mut events,
            &mut sequence,
        )
        .unwrap();
        run_checked_event(
            &manifest.runtime.git_program,
            &["commit", "-m", "Repair game"],
            &task_worktree,
            "candidate",
            &mut events,
            &mut sequence,
        )
        .unwrap();
        for args in [
            vec!["merge", "--ff-only", "benchmark-task"],
            vec!["worktree", "remove", task_worktree.to_str().unwrap()],
            vec!["branch", "-d", "benchmark-task"],
        ] {
            run_checked_event(
                &manifest.runtime.git_program,
                &args,
                &workspace,
                "closeout",
                &mut events,
                &mut sequence,
            )
            .unwrap();
        }

        let verification = verify_workflow(
            &manifest,
            AgentTokenMode::GitLinearSingleSession,
            "test-b001-gd-01-git",
            &workspace,
            Some(&task_worktree),
            Some(&start_proof),
            None,
        )
        .unwrap();
        assert!(verification.closed, "{:?}", verification.reasons);
        assert_eq!(verification.workspace_dirty, Some(false));
        assert_eq!(verification.git_lineage_matches_start, Some(true));
        assert!(!task_worktree.exists());
        assert_eq!(fs::read_dir(&worktree_container).unwrap().count(), 0);
        assert!(command_output(
            &manifest.runtime.git_program,
            &["log", "--format=%s", "-1"],
            &workspace,
        )
        .unwrap()
        .contains("Repair game"));
    }

    #[test]
    fn host_managed_detached_worktree_is_ready_before_model_and_closes_afterward() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let worktree_container = temp.path().join("git-worktree-container");
        let task_worktree = worktree_container.join("task");
        let metadata = temp.path().join("git-metadata");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&worktree_container).unwrap();
        fs::write(workspace.join("game.txt"), "baseline\n").unwrap();
        prepare_empty_directory(&metadata, "Git metadata").unwrap();

        let mut manifest = test_manifest();
        manifest.ait_edit_root_mode = crate::agent_token::AgentTokenAitEditRootMode::Returned;
        manifest.git_worktree_mode = AgentTokenGitWorktreeMode::CodexAppEquivalentManaged;
        let entry = AgentTokenScheduleEntry {
            run_id: "managed-b001-gd-01-git".to_string(),
            workload_id: "GD-01".to_string(),
            mode: AgentTokenMode::GitLinearSingleSession,
            attempt: 1,
            block_index: 1,
            randomized_order: 1,
        };
        let mut events = Vec::new();
        let mut sequence = 1;
        bootstrap_git(&manifest, &workspace, &metadata, &mut events, &mut sequence).unwrap();
        let start_proof = capture_git_start_state_proof(&manifest, &entry.run_id, &workspace);
        assert!(start_proof.passed, "{:?}", start_proof.failure_reasons);
        let start_head = start_proof.head_oid.as_deref().unwrap();
        let mut lifecycle = provision_managed_git_worktree(
            &manifest,
            &entry,
            &workspace,
            &task_worktree,
            start_head,
            &mut events,
            &mut sequence,
        )
        .unwrap();
        assert!(task_worktree.is_dir());
        assert!(command_output(
            &manifest.runtime.git_program,
            &["symbolic-ref", "--short", "HEAD"],
            &task_worktree,
        )
        .is_err());
        lifecycle.model_started_at = Some(Utc::now().to_rfc3339());
        fs::write(task_worktree.join("game.txt"), "repaired\n").unwrap();
        lifecycle.model_completed_at = Some(Utc::now().to_rfc3339());
        lifecycle.model_elapsed_ms = Some(7);
        let mut closeout_events = Vec::new();
        let mut closeout_sequence = 1;
        close_managed_git_worktree(
            &manifest,
            &workspace,
            &task_worktree,
            &mut lifecycle,
            true,
            &mut closeout_events,
            &mut closeout_sequence,
        )
        .unwrap();

        assert!(lifecycle.closed, "{:?}", lifecycle.failure_reasons);
        assert!(!lifecycle.desktop_private_ipc_invoked);
        assert_eq!(
            lifecycle.worktree_ready_head_oid.as_deref(),
            Some(start_head)
        );
        assert_eq!(lifecycle.candidate_head_oid, lifecycle.final_main_head_oid);
        assert!(!task_worktree.exists());
        assert_eq!(closeout_events.len(), 4);
        let verification = verify_workflow(
            &manifest,
            entry.mode,
            &entry.run_id,
            &workspace,
            Some(&task_worktree),
            Some(&start_proof),
            Some(&lifecycle),
        )
        .unwrap();
        assert!(verification.closed, "{:?}", verification.reasons);
        assert_eq!(verification.workflow_mode, "codex_app_equivalent_managed");
    }

    #[test]
    fn model_request_state_is_recomputed_without_inventing_provider_request_count() {
        let temp = tempfile::tempdir().unwrap();
        let codex_stream = temp.path().join("codex.jsonl");
        fs::write(
            &codex_stream,
            concat!(
                "{\"type\":\"thread.started\",\"thread_id\":\"thread-1\"}\n",
                "{\"type\":\"turn.started\"}\n",
                "{\"type\":\"model.rerouted\"}\n",
                "{\"type\":\"model.safety_buffering.updated\"}\n",
                "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}\n"
            ),
        )
        .unwrap();
        let state = inspect_model_request_state(&test_manifest(), &codex_stream, "run-1").unwrap();
        assert_eq!(state.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(state.turn_started_count, 1);
        assert_eq!(state.turn_completed_count, 1);
        assert_eq!(state.usage_event_count, 1);
        assert_eq!(state.model_reroute_event_count, 1);
        assert_eq!(state.safety_event_count, 1);
        assert_eq!(state.terminal_state, "completed");
        assert_eq!(state.internal_provider_request_count, None);
        assert!(state
            .internal_provider_request_count_authority
            .contains("turn count is not a provider-request count"));

        let claude_stream = temp.path().join("claude.jsonl");
        fs::write(
            &claude_stream,
            concat!(
                "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"session-1\"}\n",
                "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"fallback\"}]}}\n",
                "{\"type\":\"result\",\"is_error\":false,\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}\n"
            ),
        )
        .unwrap();
        let mut claude_manifest = test_manifest();
        claude_manifest.runtime.executor = crate::agent_token::AgentTokenExecutor::Claude;
        let state = inspect_model_request_state(&claude_manifest, &claude_stream, "run-2").unwrap();
        assert_eq!(state.thread_id.as_deref(), Some("session-1"));
        assert_eq!(state.model_reroute_event_count, 1);
        assert_eq!(state.terminal_state, "completed");
    }

    #[test]
    fn provider_failure_before_candidate_execution_is_classified_for_fail_fast() {
        let temp = tempfile::tempdir().unwrap();
        let raw_events = temp.path().join("codex-events.raw.jsonl");
        let stderr = temp.path().join("codex.stderr.txt");
        fs::write(
            &raw_events,
            concat!(
                "{\"type\":\"thread.started\",\"thread_id\":\"test\"}\n",
                "{\"type\":\"turn.started\"}\n",
                "{\"type\":\"error\",\"message\":\"You've hit your usage limit.\"}\n",
                "{\"type\":\"turn.failed\",\"error\":{\"message\":\"Purchase more credits.\"}}\n"
            ),
        )
        .unwrap();
        fs::write(&stderr, "").unwrap();
        let transcript = AgentTokenCommandTranscript {
            contract: crate::AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
            run_id: "test".to_string(),
            mode: AgentTokenMode::GitLinearSingleSession,
            accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            command_count: 0,
            commands: Vec::new(),
            valid: false,
            errors: Vec::new(),
            observed_required_commands: Vec::new(),
        };
        let process = TimedProcessResult {
            exit_code: Some(1),
            timed_out: false,
            elapsed_ms: 1,
        };

        assert_eq!(
            classify_codex_infrastructure_failure(
                &raw_events,
                &stderr,
                &process,
                &transcript,
                None,
            )
            .as_deref(),
            Some("provider_usage_limit")
        );
    }

    #[test]
    fn recovered_provider_transport_failure_overrides_success_usage_and_transcript() {
        let temp = tempfile::tempdir().unwrap();
        let raw_events = temp.path().join("codex-events.raw.jsonl");
        let stderr = temp.path().join("codex.stderr.txt");
        fs::write(
            &raw_events,
            concat!(
                "{\"type\":\"thread.started\",\"thread_id\":\"test\"}\n",
                "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_1\",",
                "\"type\":\"command_execution\",\"command\":\"pwd\",",
                "\"aggregated_output\":\"/tmp/workspace\\n\",\"exit_code\":0,",
                "\"status\":\"completed\"}}\n",
                "{\"type\":\"error\",\"message\":\"Reconnecting... 1/5 ",
                "(stream disconnected before completion: failed to lookup address information)\"}\n",
                "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_2\",",
                "\"type\":\"error\",\"message\":\"Falling back from WebSockets to HTTPS ",
                "transport. stream disconnected before completion\"}}\n",
                "{\"type\":\"turn.completed\",\"usage\":{",
                "\"input_tokens\":10,\"output_tokens\":2}}\n"
            ),
        )
        .unwrap();
        fs::write(&stderr, "").unwrap();
        let transcript = AgentTokenCommandTranscript {
            contract: crate::AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
            run_id: "test".to_string(),
            mode: AgentTokenMode::GitLinearSingleSession,
            accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            command_count: 1,
            commands: vec!["pwd".to_string()],
            valid: true,
            errors: Vec::new(),
            observed_required_commands: Vec::new(),
        };
        let process = TimedProcessResult {
            exit_code: Some(0),
            timed_out: false,
            elapsed_ms: 1,
        };
        let usage = test_normalized_usage();

        assert_eq!(
            classify_codex_infrastructure_failure(
                &raw_events,
                &stderr,
                &process,
                &transcript,
                Some(&usage),
            )
            .as_deref(),
            Some("provider_transport_failure")
        );
    }

    #[test]
    fn claude_classifier_reads_result_errors_and_stays_quiet_on_success() {
        let temp = tempfile::tempdir().unwrap();
        let raw_events = temp.path().join("claude-events.raw.jsonl");
        let stderr = temp.path().join("claude.stderr.txt");
        let transcript = AgentTokenCommandTranscript {
            contract: crate::AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
            run_id: "test".to_string(),
            mode: AgentTokenMode::GitLinearSingleSession,
            accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            command_count: 0,
            commands: Vec::new(),
            valid: false,
            errors: Vec::new(),
            observed_required_commands: Vec::new(),
        };
        let failed = TimedProcessResult {
            exit_code: Some(1),
            timed_out: false,
            elapsed_ms: 1,
        };

        fs::write(
            &raw_events,
            concat!(
                "{\"type\":\"system\",\"subtype\":\"init\"}\n",
                "{\"type\":\"result\",\"subtype\":\"error_during_execution\",",
                "\"is_error\":true,\"result\":\"Claude usage limit reached\"}\n"
            ),
        )
        .unwrap();
        fs::write(&stderr, "").unwrap();
        assert_eq!(
            classify_claude_infrastructure_failure(
                &raw_events,
                &stderr,
                &failed,
                &transcript,
                None,
            )
            .as_deref(),
            Some("provider_usage_limit")
        );

        // The captured r3 lane died on the subscription session limit with a
        // success subtype and is_error=true; it must classify as usage limit.
        fs::write(
            &raw_events,
            concat!(
                "{\"type\":\"system\",\"subtype\":\"init\"}\n",
                "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":true,",
                "\"result\":\"You've hit your session limit - resets 12pm (Asia/Taipei)\"}\n"
            ),
        )
        .unwrap();
        fs::write(&stderr, "").unwrap();
        assert_eq!(
            classify_claude_infrastructure_failure(
                &raw_events,
                &stderr,
                &failed,
                &transcript,
                None,
            )
            .as_deref(),
            Some("provider_usage_limit")
        );

        // A clean success result must never classify, even with stderr noise.
        fs::write(
            &raw_events,
            concat!(
                "{\"type\":\"system\",\"subtype\":\"init\"}\n",
                "{\"type\":\"result\",\"subtype\":\"success\",",
                "\"is_error\":false,\"result\":\"done\"}\n"
            ),
        )
        .unwrap();
        fs::write(&stderr, "harmless warning: rate limit banner cached").unwrap();
        let succeeded = TimedProcessResult {
            exit_code: Some(0),
            timed_out: false,
            elapsed_ms: 1,
        };
        assert_eq!(
            classify_claude_infrastructure_failure(
                &raw_events,
                &stderr,
                &succeeded,
                &transcript,
                None,
            ),
            None
        );

        // A provider refusal is successful model behavior, not unavailable
        // infrastructure. The lane remains token-accounted and is rejected by
        // the functional outcome gate instead.
        fs::write(
            &raw_events,
            concat!(
                "{\"type\":\"system\",\"subtype\":\"init\"}\n",
                "{\"type\":\"result\",\"subtype\":\"success\",",
                "\"is_error\":false,\"stop_reason\":\"refusal\",\"result\":\"cannot comply\"}\n"
            ),
        )
        .unwrap();
        fs::write(&stderr, "").unwrap();
        assert_eq!(
            classify_claude_infrastructure_failure(
                &raw_events,
                &stderr,
                &succeeded,
                &transcript,
                Some(&test_normalized_usage()),
            ),
            None
        );

        // A session that dies before emitting any terminal result event and
        // before executing any candidate command fails closed.
        fs::write(&raw_events, "{\"type\":\"system\",\"subtype\":\"init\"}\n").unwrap();
        fs::write(&stderr, "").unwrap();
        assert_eq!(
            classify_claude_infrastructure_failure(
                &raw_events,
                &stderr,
                &failed,
                &transcript,
                None,
            )
            .as_deref(),
            Some("provider_session_failed_before_candidate_execution")
        );
    }

    #[test]
    fn claude_preflight_inspector_admits_the_captured_opus_evidence() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude-opus-preflight-sample.jsonl");
        let observation = inspect_claude_executor_preflight_events(&fixture);
        assert_eq!(observation.started_command_count, 30);
        assert_eq!(observation.observed_command_count, 30);
        assert_eq!(observation.distinct_command_count, 30);
        assert_eq!(observation.successful_command_count, 30);
        assert_eq!(observation.failed_command_count, 0);
        assert_eq!(observation.unexpected_command_count, 0);
        assert_eq!(observation.unexpected_tool_item_count, 0);
        assert_eq!(observation.sequential_violation_count, 0);
        assert_eq!(observation.file_change_item_count, 0);
        assert!(observation.errors.is_empty());
    }

    #[test]
    fn claude_command_pins_isolation_sandbox_and_exact_tool_surface() {
        let mut manifest = test_manifest();
        manifest.runtime.executor = crate::agent_token::AgentTokenExecutor::Claude;
        manifest.runtime.claude_program = Some(PathBuf::from("claude"));
        manifest.model.provider = "anthropic".to_string();
        let workspace = tempfile::tempdir().unwrap();
        let metadata_root = workspace.path().join("external-metadata");
        std::fs::create_dir(&metadata_root).unwrap();
        let git_pointer = workspace.path().join("main-git-pointer");
        std::fs::write(&git_pointer, "gitdir: elsewhere\n").unwrap();
        let add_dirs = vec![metadata_root.clone()];
        let git_write_exceptions = vec![git_pointer.clone()];
        let command = build_claude_command(
            &manifest,
            workspace.path(),
            &add_dirs,
            &git_write_exceptions,
        )
        .expect("claude command");
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let pair = |flag: &str| {
            let index = args.iter().position(|arg| arg == flag).unwrap();
            args[index + 1].clone()
        };
        assert_eq!(pair("--setting-sources"), "project");
        assert_eq!(pair("--prompt-suggestions"), "false");
        // Claude Code otherwise adds one auxiliary claude-haiku-4-5 call per
        // session, which makes the terminal modelUsage inventory contain two
        // models and fails every measured lane closed on model purity.
        let envs = command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|value| {
                    (
                        key.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect::<Vec<_>>();
        assert!(
            envs.contains(&(
                crate::agent_token::CLAUDE_SINGLE_MODEL_ENV.0.to_string(),
                crate::agent_token::CLAUDE_SINGLE_MODEL_ENV.1.to_string(),
            )),
            "measured Claude surface must pin the single-model environment: {envs:?}"
        );
        let settings =
            serde_json::from_str::<serde_json::Value>(&pair("--settings")).expect("settings JSON");
        assert_eq!(settings["sandbox"]["enabled"], serde_json::json!(true));
        assert_eq!(
            settings["sandbox"]["allowUnsandboxedCommands"],
            serde_json::json!(false)
        );
        assert_eq!(
            settings["sandbox"]["network"]["allowLocalBinding"],
            serde_json::json!(true)
        );
        assert_eq!(
            settings["sandbox"]["filesystem"]["allowWrite"],
            serde_json::json!([
                metadata_root.to_str().unwrap(),
                git_pointer.to_str().unwrap(),
            ])
        );
        assert_eq!(pair("--tools"), "Bash,Read,Grep,Glob,Edit,Write");
        assert!(args.iter().any(|arg| arg == "--no-session-persistence"));
        assert!(args.iter().any(|arg| arg == "--strict-mcp-config"));
        // Only the existing directory may appear as --add-dir; the gitfile
        // write exception must stay out of --add-dir while remaining in the
        // sandbox allowWrite list asserted above.
        let add_dir_values = args
            .iter()
            .enumerate()
            .filter(|(_, arg)| *arg == "--add-dir")
            .map(|(index, _)| args[index + 1].clone())
            .collect::<Vec<_>>();
        assert_eq!(add_dir_values, vec![metadata_root.to_str().unwrap()]);
    }

    #[test]
    fn executor_feature_override_evidence_is_executor_specific() {
        let mut manifest = test_manifest();
        manifest.runtime.executor = crate::agent_token::AgentTokenExecutor::Codex;
        assert!(executor_enabled_feature_overrides(&manifest).is_empty());
        assert!(executor_disabled_feature_overrides(&manifest).is_empty());
        manifest.runtime.executor = crate::agent_token::AgentTokenExecutor::Claude;
        assert_eq!(
            executor_enabled_feature_overrides(&manifest),
            vec![
                "allowed-tool:Bash".to_string(),
                "allowed-tool:Read".to_string(),
                "allowed-tool:Grep".to_string(),
                "allowed-tool:Glob".to_string(),
                "allowed-tool:Edit".to_string(),
                "allowed-tool:Write".to_string(),
            ]
        );
        assert_eq!(
            executor_disabled_feature_overrides(&manifest),
            vec![
                "disallowed-tool:WebFetch".to_string(),
                "disallowed-tool:WebSearch".to_string(),
                "disallowed-tool:Task".to_string(),
                "disallowed-tool:NotebookEdit".to_string(),
                "disallowed-tool:TodoWrite".to_string(),
                "prompt-suggestions:false".to_string(),
            ]
        );
    }

    #[test]
    fn provider_error_classifications_remain_specific_and_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let raw_events = temp.path().join("codex-events.raw.jsonl");
        let stderr = temp.path().join("codex.stderr.txt");
        fs::write(&stderr, "").unwrap();
        let transcript = AgentTokenCommandTranscript {
            contract: crate::AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
            run_id: "test".to_string(),
            mode: AgentTokenMode::GitLinearSingleSession,
            accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            command_count: 0,
            commands: Vec::new(),
            valid: false,
            errors: Vec::new(),
            observed_required_commands: Vec::new(),
        };
        let process = TimedProcessResult {
            exit_code: Some(1),
            timed_out: false,
            elapsed_ms: 1,
        };
        let cases = [
            (
                "Too many requests from the rate limit",
                "provider_rate_limit",
            ),
            (
                "Unauthorized: log in again",
                "provider_authentication_failure",
            ),
            (
                "Requested model is unavailable",
                "provider_model_unavailable",
            ),
            (
                "Unexpected provider response",
                "provider_session_failed_before_candidate_execution",
            ),
        ];

        for (message, expected) in cases {
            fs::write(
                &raw_events,
                format!("{{\"type\":\"error\",\"message\":{message:?}}}\n"),
            )
            .unwrap();
            assert_eq!(
                classify_codex_infrastructure_failure(
                    &raw_events,
                    &stderr,
                    &process,
                    &transcript,
                    None,
                )
                .as_deref(),
                Some(expected),
                "message {message:?}"
            );
        }
    }

    #[test]
    fn ordinary_command_and_patch_errors_do_not_become_provider_failures() {
        let temp = tempfile::tempdir().unwrap();
        let raw_events = temp.path().join("codex-events.raw.jsonl");
        let stderr = temp.path().join("codex.stderr.txt");
        fs::write(
            &raw_events,
            concat!(
                "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_1\",",
                "\"type\":\"command_execution\",\"command\":\"false\",",
                "\"aggregated_output\":\"command failed\",\"exit_code\":1,",
                "\"status\":\"failed\"}}\n",
                "{\"type\":\"turn.completed\",\"usage\":{",
                "\"input_tokens\":10,\"output_tokens\":2}}\n"
            ),
        )
        .unwrap();
        fs::write(
            &stderr,
            "ERROR codex_core::tools::apply_patch: error=patch rejected: context mismatch\n",
        )
        .unwrap();
        let transcript = AgentTokenCommandTranscript {
            contract: crate::AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
            run_id: "test".to_string(),
            mode: AgentTokenMode::GitLinearSingleSession,
            accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            command_count: 1,
            commands: vec!["false".to_string()],
            valid: true,
            errors: Vec::new(),
            observed_required_commands: Vec::new(),
        };
        let process = TimedProcessResult {
            exit_code: Some(0),
            timed_out: false,
            elapsed_ms: 1,
        };
        let usage = test_normalized_usage();

        assert_eq!(
            classify_codex_infrastructure_failure(
                &raw_events,
                &stderr,
                &process,
                &transcript,
                Some(&usage),
            ),
            None
        );
    }

    #[test]
    fn recovered_tool_process_spawn_failure_is_measured_agent_behavior() {
        let temp = tempfile::tempdir().unwrap();
        let raw_events = temp.path().join("codex-events.raw.jsonl");
        let stderr = temp.path().join("codex.stderr.txt");
        fs::write(
            &raw_events,
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}\n",
        )
        .unwrap();
        fs::write(
            &stderr,
            concat!(
                "ERROR codex_core::tools::router: error=exec_command failed for `/bin/zsh -lc pwd`: ",
                "CreateProcess { message: \"Rejected(\\\"Failed to create unified exec process: ",
                "No such file or directory (os error 2)\\\")\" }\n"
            ),
        )
        .unwrap();
        let transcript = AgentTokenCommandTranscript {
            contract: crate::AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
            run_id: "test".to_string(),
            mode: AgentTokenMode::GitLinearSingleSession,
            accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            command_count: 1,
            commands: vec!["/bin/zsh -lc pwd".to_string()],
            valid: true,
            errors: Vec::new(),
            observed_required_commands: Vec::new(),
        };
        let process = TimedProcessResult {
            exit_code: Some(0),
            timed_out: false,
            elapsed_ms: 1,
        };
        let usage = test_normalized_usage();

        assert_eq!(
            classify_codex_infrastructure_failure(
                &raw_events,
                &stderr,
                &process,
                &transcript,
                Some(&usage),
            ),
            None
        );

        let timed_out = TimedProcessResult {
            exit_code: Some(0),
            timed_out: true,
            elapsed_ms: 1,
        };
        assert_eq!(
            classify_codex_infrastructure_failure(
                &raw_events,
                &stderr,
                &timed_out,
                &transcript,
                Some(&usage),
            )
            .as_deref(),
            Some("codex_tool_process_spawn_failure")
        );
        let failed = TimedProcessResult {
            exit_code: Some(1),
            timed_out: false,
            elapsed_ms: 1,
        };
        assert_eq!(
            classify_codex_infrastructure_failure(
                &raw_events,
                &stderr,
                &failed,
                &transcript,
                Some(&usage),
            )
            .as_deref(),
            Some("codex_tool_process_spawn_failure")
        );
        assert_eq!(
            classify_codex_infrastructure_failure(
                &raw_events,
                &stderr,
                &process,
                &transcript,
                None,
            )
            .as_deref(),
            Some("codex_tool_process_spawn_failure")
        );
        let empty_transcript = AgentTokenCommandTranscript {
            command_count: 0,
            commands: Vec::new(),
            valid: false,
            ..transcript
        };
        assert_eq!(
            classify_codex_infrastructure_failure(
                &raw_events,
                &stderr,
                &process,
                &empty_transcript,
                Some(&usage),
            )
            .as_deref(),
            Some("codex_tool_process_spawn_failure")
        );
    }

    #[test]
    fn non_unified_executor_spawn_failure_is_detected_from_captured_stderr_and_raw_event() {
        let temp = tempfile::tempdir().unwrap();
        let raw_events = temp.path().join("codex-events.raw.jsonl");
        let stderr = temp.path().join("codex.stderr.txt");
        fs::write(
            &raw_events,
            concat!(
                "{\"type\":\"item.completed\",\"item\":{",
                "\"id\":\"item_2\",\"type\":\"command_execution\",",
                "\"command\":\"/bin/zsh -lc 'ait task start'\",",
                "\"aggregated_output\":\"execution error: Io(Os { code: 2, kind: NotFound, message: \\\"No such file or directory\\\" })\",",
                "\"exit_code\":-1,\"status\":\"failed\"}}\n"
            ),
        )
        .unwrap();
        fs::write(
            &stderr,
            concat!(
                "ERROR codex_core::exec: exec error: No such file or directory (os error 2)\n",
                "ERROR codex_core::tools::router: error=execution error: Io(Os { code: 2, kind: NotFound, message: \"No such file or directory\" })\n"
            ),
        )
        .unwrap();

        assert!(codex_tool_process_spawn_failed(&raw_events, &stderr));
        fs::write(&stderr, "").unwrap();
        assert!(
            codex_tool_process_spawn_failed(&raw_events, &stderr),
            "the normalized raw exit-code-minus-one event must independently fail closed"
        );
    }

    #[test]
    fn rejected_apply_patch_attempts_are_counted_from_stderr() {
        let temp = tempfile::tempdir().unwrap();
        let stderr = temp.path().join("codex.stderr.txt");
        fs::write(
            &stderr,
            concat!(
                "ERROR codex_core::tools::router: error=apply_patch verification failed: first\n",
                "2026-08-23T14:45:41Z ERROR codex_core::tools::router: error=patch rejected: writing outside of the project\n",
                "WARN unrelated\n",
                "ERROR codex_core::tools::router: error=apply_patch verification failed: second\n",
                "ERROR codex_core::tools::router: error=patch rejected: another rejected attempt\n",
                "WARN codex_core::tools::router: error=patch rejected: warning is not an attempt\n",
                "ERROR other_component: error=patch rejected: unrelated component\n",
                "ERROR codex_core::tools::router: error=patch rejected without colon\n",
                "plain prose says patch rejected: but is not a router error\n"
            ),
        )
        .unwrap();
        assert_eq!(count_rejected_apply_patch_attempts(&stderr).unwrap(), 4);
    }

    #[test]
    fn evidence_output_refuses_nonempty_directory() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("existing"), "evidence").unwrap();
        let error = prepare_empty_directory(temp.path(), "campaign").unwrap_err();
        assert!(error.contains("never overwritten"));
    }

    #[test]
    fn relative_campaign_output_is_resolved_before_candidate_paths_are_derived() {
        let current = std::env::current_dir().unwrap();
        let temp = tempfile::tempdir_in(&current).unwrap();
        let requested = temp.path().join("relative-evidence");
        let relative = requested.strip_prefix(&current).unwrap();

        let resolved = prepare_campaign_output_directory(relative).unwrap();

        assert!(resolved.is_absolute());
        assert_eq!(resolved, requested.canonicalize().unwrap());
        assert!(resolved.join("runs/test/workspace").is_absolute());
    }
}
