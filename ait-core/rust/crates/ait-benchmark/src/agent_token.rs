use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::statistics::DeterministicRng;
use crate::{sha256_digest, summarize_samples, DistributionSummary};

pub const AGENT_TOKEN_CAMPAIGN_CONTRACT: &str = "ait-agent-token-benchmark-campaign/v1";
pub const AGENT_TOKEN_SCHEDULE_CONTRACT: &str = "ait-agent-token-benchmark-schedule/v1";
pub const AGENT_TOKEN_USAGE_CONTRACT: &str = "ait-agent-token-provider-usage/v1";
pub const AGENT_TOKEN_TRANSCRIPT_CONTRACT: &str = "ait-agent-token-command-transcript/v1";
pub const AGENT_TOKEN_RUN_SUMMARY_CONTRACT: &str = "ait-agent-token-benchmark-run-summary/v1";
pub const AGENT_TOKEN_RUN_ADJUDICATION_CONTRACT: &str = "ait-agent-token-run-adjudication/v1";
pub const AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION: &str = "game-development-2026-08-24.20";
pub const AGENT_TOKEN_LEGACY_ADJUDICATOR_REVISION: &str = "game-development-2026-08-24.21";
pub const AGENT_TOKEN_ADJUDICATOR_REVISION: &str = "game-development-2026-08-29.35";
/// Exact complete-campaign predecessor whose frozen schedule may be continued
/// by the successor controller without reclassifying or retrying any existing
/// valid candidate outcome.
pub const AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION: &str =
    "game-development-2026-08-26.27";
pub const AGENT_TOKEN_PRE_REPLACEMENT_PROTOCOL_REVISION: &str = "game-development-2026-08-27.28";
pub const AGENT_TOKEN_SPRINT_OFF_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-27.29";
pub const AGENT_TOKEN_PROMPTED_INSPECTION_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-28.30";
pub const AGENT_TOKEN_NATURAL_INSPECTION_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-28.31";
pub const AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-28.32";
pub const AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-28.33";
pub const AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-29.34";
pub const AGENT_TOKEN_MODEL_PURITY_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-29.35";
/// Exact predecessor whose Git validity rule still forced at least one
/// read-only discovery invocation. It has no executed source evidence and is
/// retained only so committed predecessor artifacts stay readable.
pub const AGENT_TOKEN_FORCED_GIT_DISCOVERY_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-29.37";
/// Exact predecessor whose frozen Claude executor surface still admitted the
/// auxiliary non-pinned model call. Its only artifact is one stopped zero-lane
/// preflight failure, so it carries no measured source evidence.
pub const AGENT_TOKEN_AUXILIARY_MODEL_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-29.38";
/// Exact predecessor whose measured AIT workspace still carried the generated
/// AIT workflow guidance block. Its evidence is one ten-lane Claude smoke.
pub const AGENT_TOKEN_PROJECT_DOCUMENT_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-29.39";
/// Exact predecessor whose measured AIT prompt still required the agent to read
/// the returned `edit_root` before it could enter its Task worktree.
pub const AGENT_TOKEN_IMPLICIT_EDIT_ROOT_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-30.40";
/// Exact predecessor whose prompts handed over the edit root but phrased the
/// chained start-and-enter form as optional; zero measured lanes chained.
pub const AGENT_TOKEN_OPTIONAL_CHAIN_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-30.41";
/// Exact predecessor whose measured AIT workspace still listed AGENTS.md and
/// docs/, charging AIT an exploration tax the Git workspace never pays.
pub const AGENT_TOKEN_ARTIFACT_TAX_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-30.42";
/// Exact predecessor with the clean workspace but no guidance delivery; its
/// evidence is one gated pair with zero artifact references.
pub const AGENT_TOKEN_BARE_WORKSPACE_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-30.43";
/// Exact predecessor before edit-root delivery became a declared manifest axis.
pub const AGENT_TOKEN_FIXED_EDIT_ROOT_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-30.44";
/// Exact predecessor before model admission became a declared manifest axis.
pub const AGENT_TOKEN_STRICT_ONLY_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-31.45";
/// Exact predecessor that introduced the Claude model-admission axis. `.46`
/// campaigns remain resumable with their frozen agent-owned Git lifecycle.
pub const AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-31.46";
/// Exact predecessor that introduced the separately pinned host-managed
/// worktree axis. Its two launches stopped before any measured model lane: the
/// first on copied-fixture permissions and the second on an over-broad Codex
/// Git-write exception. The immutable zero-lane evidence remains readable,
/// while `.48` corrects the permission boundary and is the only active form.
pub const AGENT_TOKEN_MANAGED_WORKTREE_PREFLIGHT_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-31.47";
pub const AGENT_TOKEN_200_SESSION_PREDECESSOR_PROTOCOL_REVISION: &str =
    "game-development-2026-08-26.26";
/// Complete-scope predecessor revisions that remain readable as frozen
/// evidence; campaigns cannot start under them. The runner admits only the
/// explicitly enumerated narrow continuation and recovery exceptions.
pub const AGENT_TOKEN_COMPLETE_PREDECESSOR_PROTOCOL_REVISIONS: &[&str] = &[
    AGENT_TOKEN_MANAGED_WORKTREE_PREFLIGHT_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_STRICT_ONLY_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_FIXED_EDIT_ROOT_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_BARE_WORKSPACE_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_ARTIFACT_TAX_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_OPTIONAL_CHAIN_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_IMPLICIT_EDIT_ROOT_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_PROJECT_DOCUMENT_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_AUXILIARY_MODEL_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_FORCED_GIT_DISCOVERY_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_MODEL_PURITY_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_PROMPTED_INSPECTION_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_NATURAL_INSPECTION_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_SPRINT_OFF_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_PRE_REPLACEMENT_PROTOCOL_REVISION,
    AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION,
    AGENT_TOKEN_200_SESSION_PREDECESSOR_PROTOCOL_REVISION,
    "game-development-2026-08-26.25",
    "game-development-2026-08-25.24",
    "game-development-2026-08-25.23",
    "game-development-2026-08-25.22",
];
pub const AGENT_TOKEN_REPORT_CONTRACT: &str = "ait-agent-token-benchmark-report/v3";
pub const AGENT_TOKEN_ENVIRONMENT_CONTRACT: &str = "ait-agent-token-benchmark-environment/v1";
pub const AGENT_TOKEN_BROWSER_REPORT_CONTRACT: &str = "ait-agent-token-browser-report/v1";
pub const AGENT_TOKEN_PROTOCOL_REVISION: &str = "game-development-2026-08-31.48";
pub const AGENT_TOKEN_RECOVERED_SPAWN_POLICY_REVISION: &str = "game-development-2026-08-29.35";
pub const AGENT_TOKEN_RECOVERED_SPAWN_CAMPAIGN_ID: &str =
    "game-v1-g56s-max-sprint-on-natural-complete200-20260828";
pub const AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID: &str =
    "game-v1-g56s-max-sprint-on-natural-complete200-20260828-b017-gd-03-git";
pub const AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX: usize = 160;
pub const AGENT_TOKEN_RECOVERED_SPAWN_SOURCE_SUMMARY_SHA256: &str =
    "sha256:0a58cb77970cb4d41a531a99b69fc8acdde4e7970b2327a14333ad050a9f0dac";
pub const AGENT_TOKEN_RECOVERED_SPAWN_REASON: &str = "Repository-owner-authorized semantic correction: a recovered candidate tool-process launch mistake is measured agent behavior, not unavailable infrastructure";
pub const AGENT_TOKEN_RECOVERED_SPAWN_PAIR_ADMISSION_POLICY: &str = "exact_protocol_valid_pairs_with_transparent_infrastructure_and_host_shutdown_whole_pair_recovery_plus_digest_linked_recovered_spawn_adjudication";
/// Read-only repository inspection subcommands admitted as informational in
/// the measured AIT lifecycle without naming or recommending them in the
/// measured prompt. Token cost stays measured while the invocation is neither
/// a lifecycle step nor a forbidden surface.
pub const AIT_INFORMATIONAL_INSPECTION_SUBCOMMANDS: &[&[&str]] =
    &[&["status"], &["diff"], &["blame"]];

/// The exact built-in tool surface available to measured Claude lanes; the
/// runner pins availability with `--tools` and the transcript validator
/// fails closed on any tool use outside this set.
pub const CLAUDE_MEASURED_TOOL_SURFACE: &[&str] =
    &["Bash", "Read", "Grep", "Glob", "Edit", "Write"];

/// Pins a measured Claude session to exactly one model. Claude Code otherwise
/// issues one small auxiliary `claude-haiku-4-5` call per session, which makes
/// the terminal modelUsage inventory contain two models and fails every lane
/// closed under the model-purity rule. Bisected against the exact measured flag
/// set: `DISABLE_NON_ESSENTIAL_MODEL_CALLS` has no effect and
/// `CLAUDE_CODE_SIMPLE` yields no usable stream, while this variable leaves the
/// success, stop-reason, turn, and tool-call contract intact. It is applied
/// identically to the executor preflight and to both measured modes.
pub const CLAUDE_SINGLE_MODEL_ENV: (&str, &str) = ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1");

/// The measured AIT workspace keeps a content-free project document. `ait init`
/// and `ait config set` generate an AGENTS.md carrying the effective AIT
/// workflow block, which no measured Git workspace has. The file cannot be
/// deleted: it is tracked authored Markdown, so removal makes `ait plan sync`
/// fail with a missing path and `ait task start` refuse on plan drift.
/// Overwriting it with this stub and reconciling through a local plan sync
/// keeps the whole lifecycle working while removing the guidance content.
/// The generated project document removed from every measured AIT workspace.
/// Deletion plus `ait plan sync AGENTS.md --prune --local` archives its Plan
/// and the whole lifecycle keeps working; the earlier stub belief that the
/// file could not be deleted was wrong.
pub const AIT_PURGED_PROJECT_DOCUMENT: &str = "AGENTS.md";

pub const AGENT_TOKEN_PAIR_ADMISSION_POLICY: &str =
    "exact_protocol_valid_pair_without_workflow_metric_exclusion";
const AGENT_TOKEN_COMPLETE_ATTEMPTS_PER_WORKLOAD: usize = 20;
const AGENT_TOKEN_COMPLETE_SCHEDULED_RUNS: usize = 200;
const AGENT_TOKEN_PREDECESSOR_COMPLETE_ATTEMPTS_PER_WORKLOAD: usize = 10;
const GIT_METADATA_CONTEXT_OVERRIDE_ERROR: &str =
    "Git mode overrode the runner-owned isolated repository metadata context";

pub(crate) fn protocol_requires_claude_model_evidence(protocol_revision: &str) -> bool {
    matches!(
        protocol_revision,
        AGENT_TOKEN_PROTOCOL_REVISION
            | AGENT_TOKEN_MANAGED_WORKTREE_PREFLIGHT_PREDECESSOR_PROTOCOL_REVISION
            | AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION
            | AGENT_TOKEN_STRICT_ONLY_PREDECESSOR_PROTOCOL_REVISION
            | AGENT_TOKEN_FIXED_EDIT_ROOT_PREDECESSOR_PROTOCOL_REVISION
            | AGENT_TOKEN_BARE_WORKSPACE_PREDECESSOR_PROTOCOL_REVISION
            | AGENT_TOKEN_ARTIFACT_TAX_PREDECESSOR_PROTOCOL_REVISION
            | AGENT_TOKEN_OPTIONAL_CHAIN_PREDECESSOR_PROTOCOL_REVISION
            | AGENT_TOKEN_IMPLICIT_EDIT_ROOT_PREDECESSOR_PROTOCOL_REVISION
            | AGENT_TOKEN_PROJECT_DOCUMENT_PREDECESSOR_PROTOCOL_REVISION
            | AGENT_TOKEN_AUXILIARY_MODEL_PREDECESSOR_PROTOCOL_REVISION
            | AGENT_TOKEN_FORCED_GIT_DISCOVERY_PREDECESSOR_PROTOCOL_REVISION
    )
}

fn protocol_supports_as_shipped_claude_admission(protocol_revision: &str) -> bool {
    matches!(
        protocol_revision,
        AGENT_TOKEN_PROTOCOL_REVISION | AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION
    )
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AgentTokenMode {
    GitLinearSingleSession,
    AitLinearSingleSession,
}

impl AgentTokenMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GitLinearSingleSession => "git_linear_single_session",
            Self::AitLinearSingleSession => "ait_linear_single_session",
        }
    }

    fn short_name(&self) -> &'static str {
        match self {
            Self::GitLinearSingleSession => "git",
            Self::AitLinearSingleSession => "ait",
        }
    }
}

/// Who owns the Git lane's linked-worktree lifecycle. The legacy treatment
/// charges worktree creation, commit, integration, and cleanup to the measured
/// agent. The App-equivalent treatment reproduces Codex Desktop's observable
/// boundary: a detached worktree exists before the first model event and the
/// host closes it after the turn. Desktop's private IPC is deliberately not
/// claimed or inferred.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTokenGitWorktreeMode {
    #[default]
    AgentManaged,
    CodexAppEquivalentManaged,
}

impl AgentTokenGitWorktreeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentManaged => "agent_managed",
            Self::CodexAppEquivalentManaged => "codex_app_equivalent_managed",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTokenAccountingProfile {
    SteadyStateTaskCost,
    FirstUseTotalCost,
}

impl AgentTokenAccountingProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SteadyStateTaskCost => "steady_state_task_cost",
            Self::FirstUseTotalCost => "first_use_total_cost",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTokenCampaignScope {
    Smoke,
    /// Bounded single-purpose evidence with a free attempt count and no
    /// full-matrix requirement, used to measure within-cell variance or to
    /// isolate one mechanism. Never publication eligible and never pooled.
    Diagnostic,
    Pilot,
    Qualification,
    Complete,
}

impl AgentTokenCampaignScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Smoke => "smoke",
            Self::Diagnostic => "diagnostic",
            Self::Pilot => "pilot",
            Self::Qualification => "qualification",
            Self::Complete => "complete",
        }
    }

    fn minimum_attempts(&self) -> usize {
        match self {
            Self::Smoke => 1,
            Self::Diagnostic => 2,
            Self::Pilot => 10,
            Self::Qualification => 20,
            Self::Complete => AGENT_TOKEN_COMPLETE_ATTEMPTS_PER_WORKLOAD,
        }
    }

    fn requires_full_workload_matrix(&self) -> bool {
        !matches!(self, Self::Smoke | Self::Diagnostic)
    }

    /// Diagnostic campaigns choose their own attempt count so within-cell
    /// variance can be measured; every other scope pins an exact count.
    fn pins_exact_attempts(&self) -> bool {
        !matches!(self, Self::Diagnostic)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTokenAitSprintMode {
    #[default]
    Off,
    On,
}

impl AgentTokenAitSprintMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
        }
    }
}

/// How the measured AIT prompt delivers the Task worktree location. Campaigns
/// cannot start under a frozen predecessor revision, so both variants live
/// under one active revision and are selected per campaign as a pinned axis.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTokenAitEditRootMode {
    /// The prompt supplies the benchmark-owned absolute path and the canonical
    /// chained start-and-enter command, mirroring the Git prompt.
    #[default]
    Explicit,
    /// The prompt withholds the path and instructs the agent to enter the
    /// `edit_root` returned by Task start, as protocol .40 did.
    Returned,
}

impl AgentTokenAitEditRootMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Returned => "returned",
        }
    }
}

/// How a measured Claude session treats a provider-side model substitution.
/// The runner never supplies `--fallback-model`, yet the provider can still
/// emit a fallback event mid-session and serve the remainder on another model.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTokenClaudeModelAdmission {
    /// Any model outside the pin fails the lane closed.
    #[default]
    Strict,
    /// Measure Claude Code as shipped: admit the session when the pinned model
    /// is present, and sum every terminal modelUsage entry so no
    /// provider-reported token is dropped. The per-model composition and any
    /// fallback events remain verbatim in the retained raw stream.
    AsShipped,
}

impl AgentTokenClaudeModelAdmission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::AsShipped => "as_shipped",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTokenCampaignManifest {
    pub contract: String,
    pub campaign_id: String,
    pub protocol_revision: String,
    pub campaign_scope: AgentTokenCampaignScope,
    pub accounting_profile: AgentTokenAccountingProfile,
    #[serde(default)]
    pub ait_sprint_mode: AgentTokenAitSprintMode,
    #[serde(default)]
    pub ait_edit_root_mode: AgentTokenAitEditRootMode,
    #[serde(default)]
    pub git_worktree_mode: AgentTokenGitWorktreeMode,
    #[serde(default)]
    pub claude_model_admission: AgentTokenClaudeModelAdmission,
    #[serde(default)]
    pub functional_replacement_policy: AgentTokenFunctionalReplacementPolicy,
    pub seed: u64,
    pub attempts_per_cell: usize,
    pub workload_ids: Vec<String>,
    pub modes: Vec<AgentTokenMode>,
    pub model: AgentTokenModelPin,
    pub runtime: AgentTokenRuntime,
    pub cache_class: String,
    pub network_policy: String,
    pub tool_policy: String,
    #[serde(default = "default_bootstrap_resamples")]
    pub bootstrap_resamples: usize,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTokenFunctionalReplacementPolicy {
    #[default]
    None,
    FirstValidUnacceptedLaneOnce,
}

impl AgentTokenFunctionalReplacementPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::FirstValidUnacceptedLaneOnce => "first_valid_unaccepted_lane_once",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentTokenModelPin {
    pub provider: String,
    pub model_id: String,
    pub model_revision: String,
    pub reasoning_effort: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTokenRuntime {
    /// Measured agent executor. Defaults to `codex`, keeping every existing
    /// manifest byte-compatible; `claude` selects the Claude Code headless
    /// executor introduced by protocol revision `.23`.
    #[serde(default)]
    pub executor: AgentTokenExecutor,
    pub codex_program: PathBuf,
    /// Claude Code executable. Required when `executor` is `claude`; unused
    /// and optional for the codex executor.
    #[serde(default)]
    pub claude_program: Option<PathBuf>,
    /// Exact measured executor `--version` output. Protocol `.37` requires
    /// this pin for Claude campaigns and checks it before any model request.
    #[serde(default)]
    pub executor_version: Option<String>,
    #[serde(default)]
    pub ait_version: Option<String>,
    #[serde(default)]
    pub git_version: Option<String>,
    #[serde(default)]
    pub node_version: Option<String>,
    #[serde(default)]
    pub browser_version: Option<String>,
    pub ait_program: PathBuf,
    pub git_program: PathBuf,
    pub node_program: PathBuf,
    pub browser_program: Option<PathBuf>,
    pub fixture_manifest: PathBuf,
    pub run_timeout_seconds: u64,
    pub ait_first_use_worktree_add_dir: Option<PathBuf>,
    #[serde(default)]
    pub project_doc_max_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTokenExecutor {
    #[default]
    Codex,
    Claude,
}

impl AgentTokenExecutor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenSchedule {
    pub contract: String,
    pub campaign_id: String,
    pub protocol_revision: String,
    pub seed: u64,
    pub entry_count: usize,
    pub entries: Vec<AgentTokenScheduleEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenScheduleEntry {
    pub run_id: String,
    pub workload_id: String,
    pub mode: AgentTokenMode,
    pub attempt: usize,
    pub block_index: usize,
    pub randomized_order: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NormalizedAgentTokenUsage {
    pub contract: String,
    pub run_id: String,
    pub workload_id: String,
    pub mode: AgentTokenMode,
    pub accounting_profile: AgentTokenAccountingProfile,
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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentTokenServedModelUsage {
    pub model_id: String,
    pub canonical_model: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub provider_total_tokens: u64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct AgentTokenServedModelReport {
    pub model_id: String,
    pub canonical_model: String,
    pub run_count: usize,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub provider_total_tokens: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenCommandTranscript {
    pub contract: String,
    pub run_id: String,
    pub mode: AgentTokenMode,
    pub accounting_profile: AgentTokenAccountingProfile,
    pub command_count: usize,
    pub commands: Vec<String>,
    pub valid: bool,
    pub errors: Vec<String>,
    pub observed_required_commands: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenEnvironment {
    pub contract: String,
    pub captured_at: String,
    pub os: String,
    pub architecture: String,
    pub codex_version: String,
    pub ait_version: String,
    pub git_version: String,
    pub node_version: String,
    pub browser_version: Option<String>,
    pub workflow_mode: String,
    #[serde(default)]
    pub ait_edit_root_mode: AgentTokenAitEditRootMode,
    #[serde(default)]
    pub git_worktree_mode: AgentTokenGitWorktreeMode,
    pub sprint_mode: String,
    pub ait_server_connected: bool,
    pub network_policy: String,
    #[serde(default)]
    pub codex_permission_profile: String,
    #[serde(default)]
    pub codex_permission_profile_parent: String,
    pub cache_class: String,
    #[serde(default)]
    pub benchmark_enabled_feature_overrides: Vec<String>,
    #[serde(default)]
    pub benchmark_disabled_feature_overrides: Vec<String>,
    #[serde(default)]
    pub project_doc_max_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenBrowserReport {
    pub contract: String,
    pub workload_id: String,
    pub required_for_equivalent_completion: bool,
    pub status: String,
    pub desktop_passed: Option<bool>,
    pub mobile_passed: Option<bool>,
    pub console_errors: Option<usize>,
    pub failed_requests: Option<usize>,
    pub horizontal_overflow: Option<bool>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenRunSummary {
    pub contract: String,
    pub campaign_id: String,
    pub run_id: String,
    pub workload_id: String,
    pub mode: AgentTokenMode,
    pub accounting_profile: AgentTokenAccountingProfile,
    pub attempt: usize,
    pub block_index: usize,
    pub randomized_order: usize,
    pub initial_content_digest: String,
    pub final_content_digest: Option<String>,
    pub codex_exit_code: Option<i32>,
    pub codex_timed_out: bool,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub infrastructure_failure: Option<String>,
    #[serde(default)]
    pub provider_refusal: bool,
    #[serde(default)]
    pub provider_stop_reason: Option<String>,
    pub usage: Option<NormalizedAgentTokenUsage>,
    pub transcript: AgentTokenCommandTranscript,
    pub secondary_metrics: AgentTokenSecondaryMetrics,
    pub evaluator_exit_code: Option<i32>,
    pub evaluator_score: Option<u64>,
    pub evaluator_accepted: bool,
    pub browser: AgentTokenBrowserReport,
    pub workflow_closed: bool,
    pub valid_attempt: bool,
    pub accepted_equivalent: bool,
    pub invalid_reasons: Vec<String>,
    pub failure_reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenRunAdjudication {
    pub contract: String,
    pub campaign_id: String,
    pub run_id: String,
    pub source_protocol_revision: String,
    pub adjudicator_revision: String,
    pub source_run_summary_sha256: String,
    pub reason: String,
    pub effective_summary: AgentTokenRunSummary,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentTokenSecondaryMetrics {
    pub agent_turns: usize,
    pub model_calls: usize,
    pub command_tool_calls: usize,
    pub file_change_items: usize,
    #[serde(default)]
    pub apply_patch_rejected_attempts: usize,
    #[serde(default)]
    pub apply_patch_attempts: usize,
    pub tool_output_bytes: u64,
    pub project_validation_calls: usize,
    pub repository_query_calls: usize,
    pub repeated_repository_query_calls: usize,
    pub help_calls: usize,
    pub file_read_or_search_calls: usize,
    pub tool_calls_by_family: BTreeMap<String, usize>,
    #[serde(default)]
    pub host_worktree_provisioning_elapsed_ms: Option<u64>,
    #[serde(default)]
    pub host_worktree_closeout_elapsed_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenReport {
    pub contract: String,
    pub campaign_id: String,
    pub protocol_revision: String,
    pub campaign_scope: String,
    pub accounting_profile: String,
    #[serde(default)]
    pub ait_sprint_mode: AgentTokenAitSprintMode,
    #[serde(default)]
    pub ait_edit_root_mode: AgentTokenAitEditRootMode,
    #[serde(default)]
    pub git_worktree_mode: AgentTokenGitWorktreeMode,
    #[serde(default)]
    pub functional_replacement_policy: AgentTokenFunctionalReplacementPolicy,
    pub model: AgentTokenModelPin,
    pub cache_class: String,
    pub network_policy: String,
    #[serde(default)]
    pub project_doc_max_bytes: usize,
    #[serde(default)]
    pub pair_admission_policy: String,
    pub generated_at: String,
    pub scheduled_run_count: usize,
    pub observed_run_count: usize,
    #[serde(default)]
    pub executed_evidence_run_count: usize,
    #[serde(default)]
    pub statistically_excluded_run_count: usize,
    pub invalid_run_count: usize,
    #[serde(default)]
    pub served_models: Vec<AgentTokenServedModelReport>,
    #[serde(default)]
    pub mixed_model_run_count: usize,
    #[serde(default)]
    pub fallback_observed_run_count: usize,
    pub groups: Vec<AgentTokenGroupReport>,
    pub comparisons: Vec<AgentTokenModeComparison>,
    pub aggregate_median_token_savings_percent: Option<f64>,
    pub aggregate_token_savings_bootstrap_ci95: Option<[f64; 2]>,
    #[serde(default)]
    pub aggregate_median_elapsed_savings_percent: Option<f64>,
    #[serde(default)]
    pub aggregate_median_completed_file_change_reduction_percent: Option<f64>,
    #[serde(default)]
    pub aggregate_median_rejected_apply_patch_reduction_percent: Option<f64>,
    #[serde(default)]
    pub aggregate_median_apply_patch_attempt_reduction_percent: Option<f64>,
    #[serde(default)]
    pub source_protocol_claim_eligible: bool,
    #[serde(default)]
    pub current_policy_revision: String,
    #[serde(default)]
    pub current_policy_evaluation_mode: String,
    #[serde(default)]
    pub current_policy_criteria_met: bool,
    #[serde(default)]
    pub current_policy_blockers: Vec<String>,
    #[serde(default)]
    pub source_protocol_blockers: Vec<String>,
    #[serde(default)]
    pub replacement_policy_revision: Option<String>,
    #[serde(default)]
    pub statistical_replacements: Vec<AgentTokenStatisticalReplacementRecord>,
    #[serde(default)]
    pub infrastructure_recovery_policy_revision: Option<String>,
    #[serde(default)]
    pub infrastructure_pair_recoveries: Vec<AgentTokenInfrastructurePairRecoveryRecord>,
    #[serde(default)]
    pub host_shutdown_recovery_policy_revision: Option<String>,
    #[serde(default)]
    pub host_shutdown_pair_recoveries: Vec<AgentTokenHostShutdownPairRecoveryRecord>,
    #[serde(default)]
    pub recovered_spawn_policy_revision: Option<String>,
    #[serde(default)]
    pub recovered_spawn_adjudications: Vec<AgentTokenRecoveredSpawnAdjudicationRecord>,
    pub claim_eligible: bool,
    pub blockers: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenRecoveredSpawnAdjudicationRecord {
    pub run_id: String,
    pub source_run_summary_sha256: String,
    pub adjudicator_revision: String,
    pub source_infrastructure_failure: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenStatisticalReplacementRecord {
    pub source_run_id: String,
    pub replacement_run_id: String,
    pub source_run_summary_sha256: String,
    pub replacement_run_summary_sha256: String,
    #[serde(default)]
    pub replacement_runner_sha256: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenInfrastructurePairRecoveryRecord {
    pub source_pair_start_index: usize,
    pub workload_id: String,
    pub attempt: usize,
    pub source_schedule_run_ids: Vec<String>,
    pub observed_source_run_ids: Vec<String>,
    pub replacement_run_ids: Vec<String>,
    pub recovery_runner_sha256: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenHostShutdownPairRecoveryRecord {
    pub source_pair_start_index: usize,
    pub workload_id: String,
    pub attempt: usize,
    pub source_schedule_run_ids: Vec<String>,
    pub interrupted_run_id: String,
    pub interrupted_event_sha256: String,
    pub interrupted_event_mtime_unix_s: u64,
    pub interrupted_artifact_count: usize,
    pub host_observation_sha256: String,
    pub replacement_run_ids: Vec<String>,
    pub recovery_runner_sha256: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenGroupReport {
    pub workload_id: String,
    pub mode: String,
    pub attempted_count: usize,
    pub valid_count: usize,
    pub accepted_count: usize,
    pub invalid_count: usize,
    pub acceptance_rate: f64,
    pub total_valid_attempt_tokens: u64,
    pub effective_tokens_per_accepted_task: Option<f64>,
    pub valid_attempt_token_distribution: Option<DistributionSummary>,
    pub input_token_distribution: Option<DistributionSummary>,
    pub cached_input_token_distribution: Option<DistributionSummary>,
    pub output_token_distribution: Option<DistributionSummary>,
    pub reasoning_token_distribution: Option<DistributionSummary>,
    #[serde(default)]
    pub elapsed_ms_distribution: Option<DistributionSummary>,
    #[serde(default)]
    pub completed_file_change_item_distribution: Option<DistributionSummary>,
    #[serde(default)]
    pub rejected_apply_patch_attempt_distribution: Option<DistributionSummary>,
    #[serde(default)]
    pub apply_patch_attempt_distribution: Option<DistributionSummary>,
    #[serde(default)]
    pub host_worktree_provisioning_elapsed_ms_distribution: Option<DistributionSummary>,
    #[serde(default)]
    pub host_worktree_closeout_elapsed_ms_distribution: Option<DistributionSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenModeComparison {
    pub workload_id: String,
    pub git_effective_tokens: Option<f64>,
    pub ait_effective_tokens: Option<f64>,
    pub token_savings_percent: Option<f64>,
    pub token_savings_bootstrap_ci95: Option<[f64; 2]>,
    pub paired_valid_attempt_count: usize,
    #[serde(default)]
    pub git_effective_elapsed_ms: Option<f64>,
    #[serde(default)]
    pub ait_effective_elapsed_ms: Option<f64>,
    #[serde(default)]
    pub elapsed_savings_percent: Option<f64>,
    #[serde(default)]
    pub git_effective_completed_file_change_items: Option<f64>,
    #[serde(default)]
    pub ait_effective_completed_file_change_items: Option<f64>,
    #[serde(default)]
    pub completed_file_change_reduction_percent: Option<f64>,
    #[serde(default)]
    pub git_effective_rejected_apply_patch_attempts: Option<f64>,
    #[serde(default)]
    pub ait_effective_rejected_apply_patch_attempts: Option<f64>,
    #[serde(default)]
    pub rejected_apply_patch_reduction_percent: Option<f64>,
    #[serde(default)]
    pub git_effective_apply_patch_attempts: Option<f64>,
    #[serde(default)]
    pub ait_effective_apply_patch_attempts: Option<f64>,
    #[serde(default)]
    pub apply_patch_attempt_reduction_percent: Option<f64>,
    pub git_acceptance_rate: f64,
    pub ait_acceptance_rate: f64,
    pub acceptance_rate_deficit_percentage_points: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenCrossCampaignReport {
    pub contract: String,
    pub baseline_campaign_id: String,
    pub candidate_campaign_id: String,
    pub comparable: bool,
    pub deltas: Vec<AgentTokenCrossCampaignDelta>,
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenCrossCampaignDelta {
    pub workload_id: String,
    pub mode: String,
    pub baseline_effective_tokens: Option<f64>,
    pub candidate_effective_tokens: Option<f64>,
    pub effective_token_delta_percent: Option<f64>,
    pub baseline_acceptance_rate: f64,
    pub candidate_acceptance_rate: f64,
    pub acceptance_rate_delta_percentage_points: f64,
}

fn default_bootstrap_resamples() -> usize {
    2_000
}

pub fn load_agent_token_campaign(path: &Path) -> Result<AgentTokenCampaignManifest, String> {
    let manifest = read_agent_token_campaign(path)?;
    validate_agent_token_campaign(&manifest)?;
    Ok(manifest)
}

pub fn load_agent_token_campaign_for_evidence(
    path: &Path,
) -> Result<AgentTokenCampaignManifest, String> {
    load_agent_token_campaign_for_evidence_with_fixture_override(path, None)
}

/// Loads an evidence campaign manifest, optionally replacing the fixture
/// manifest path. The stored path is relative to the committed campaigns
/// directory and therefore does not resolve from an evidence directory, so
/// resume supplies it explicitly. Fixture content is still verified by digest
/// on every run, so an override cannot swap fixtures silently.
pub fn load_agent_token_campaign_for_evidence_with_fixture_override(
    path: &Path,
    fixture_manifest: Option<&Path>,
) -> Result<AgentTokenCampaignManifest, String> {
    let mut manifest = read_agent_token_campaign(path)?;
    if let Some(fixture) = fixture_manifest {
        manifest.runtime.fixture_manifest = fixture.to_path_buf();
    }
    validate_agent_token_campaign_source(&manifest)?;
    Ok(manifest)
}

fn read_agent_token_campaign(path: &Path) -> Result<AgentTokenCampaignManifest, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "Failed to read agent-token campaign manifest {}: {error}",
            path.display()
        )
    })?;
    let mut manifest =
        serde_json::from_slice::<AgentTokenCampaignManifest>(&bytes).map_err(|error| {
            format!(
                "Failed to decode agent-token campaign manifest {}: {error}",
                path.display()
            )
        })?;
    resolve_campaign_paths(path, &mut manifest)?;
    Ok(manifest)
}

pub fn validate_agent_token_campaign(manifest: &AgentTokenCampaignManifest) -> Result<(), String> {
    if manifest.protocol_revision != AGENT_TOKEN_PROTOCOL_REVISION {
        return Err(format!(
            "Agent-token protocol revision must be {AGENT_TOKEN_PROTOCOL_REVISION}, got {}",
            manifest.protocol_revision
        ));
    }
    if !matches!(
        manifest.campaign_scope,
        AgentTokenCampaignScope::Smoke
            | AgentTokenCampaignScope::Diagnostic
            | AgentTokenCampaignScope::Complete
    ) {
        return Err(format!(
            "Agent-token protocol {AGENT_TOKEN_PROTOCOL_REVISION} admits only smoke, diagnostic, or complete campaign scope, got {}",
            manifest.campaign_scope.as_str()
        ));
    }
    validate_agent_token_campaign_shape(manifest)
}

fn validate_agent_token_campaign_source(
    manifest: &AgentTokenCampaignManifest,
) -> Result<(), String> {
    let revision = manifest.protocol_revision.as_str();
    if revision == AGENT_TOKEN_PROTOCOL_REVISION
        || AGENT_TOKEN_COMPLETE_PREDECESSOR_PROTOCOL_REVISIONS.contains(&revision)
    {
        if !matches!(
            manifest.campaign_scope,
            AgentTokenCampaignScope::Smoke | AgentTokenCampaignScope::Complete
        ) {
            return Err(format!(
                "Agent-token protocol {} admits only smoke or complete campaign scope, got {}",
                manifest.protocol_revision,
                manifest.campaign_scope.as_str()
            ));
        }
    } else if revision == AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION {
        if !matches!(
            manifest.campaign_scope,
            AgentTokenCampaignScope::Smoke
                | AgentTokenCampaignScope::Pilot
                | AgentTokenCampaignScope::Qualification
        ) {
            return Err(format!(
                "Agent-token protocol {AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION} admits only smoke, pilot, or qualification campaign scope, got {}",
                manifest.campaign_scope.as_str()
            ));
        }
    } else {
        return Err(format!(
            "Agent-token evidence protocol revision must be {AGENT_TOKEN_PROTOCOL_REVISION}, a complete predecessor ({}), or the narrow resumable predecessor {AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION}, got {}",
            AGENT_TOKEN_COMPLETE_PREDECESSOR_PROTOCOL_REVISIONS.join(", "),
            manifest.protocol_revision
        ));
    }
    validate_agent_token_campaign_shape(manifest)
}

fn validate_agent_token_campaign_shape(
    manifest: &AgentTokenCampaignManifest,
) -> Result<(), String> {
    if manifest.contract != AGENT_TOKEN_CAMPAIGN_CONTRACT {
        return Err(format!(
            "Agent-token campaign contract must be {AGENT_TOKEN_CAMPAIGN_CONTRACT}, got {}",
            manifest.contract
        ));
    }
    require_text("campaign_id", &manifest.campaign_id)?;
    if manifest.seed == 0 {
        return Err("campaign seed must be a recorded positive integer".to_string());
    }
    if !manifest
        .campaign_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(
            "campaign_id must contain only ASCII letters, digits, hyphen, or underscore"
                .to_string(),
        );
    }
    let required_attempts = source_required_attempts_per_cell(manifest);
    if !manifest.campaign_scope.pins_exact_attempts() {
        if manifest.attempts_per_cell < required_attempts {
            return Err(format!(
                "{} campaign requires at least {} attempts per workload and mode",
                manifest.campaign_scope.as_str(),
                required_attempts
            ));
        }
    } else if manifest.attempts_per_cell != required_attempts {
        return Err(format!(
            "{} campaign requires exactly {} attempts per workload and mode",
            manifest.campaign_scope.as_str(),
            required_attempts
        ));
    }
    if manifest.bootstrap_resamples < 1_000 {
        return Err("bootstrap_resamples must be at least 1000".to_string());
    }
    let workloads = manifest
        .workload_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if workloads.len() != manifest.workload_ids.len() || workloads.is_empty() {
        return Err("workload_ids must be non-empty and unique".to_string());
    }
    for workload in &workloads {
        if !matches!(*workload, "GD-01" | "GD-02" | "GD-03" | "GD-04" | "GD-05") {
            return Err(format!("Unsupported game benchmark workload: {workload}"));
        }
    }
    if manifest.campaign_scope.requires_full_workload_matrix()
        && workloads != BTreeSet::from(["GD-01", "GD-02", "GD-03", "GD-04", "GD-05"])
    {
        return Err(format!(
            "{} campaigns must contain all five workloads",
            manifest.campaign_scope.as_str()
        ));
    }
    let modes = manifest.modes.iter().copied().collect::<BTreeSet<_>>();
    if modes.len() != manifest.modes.len() || modes.is_empty() {
        return Err("modes must be non-empty and unique".to_string());
    }
    if modes
        != BTreeSet::from([
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenMode::AitLinearSingleSession,
        ])
    {
        return Err("Core campaign modes must be exactly git_linear_single_session and ait_linear_single_session".to_string());
    }
    for (field, value) in [
        ("model.provider", manifest.model.provider.as_str()),
        ("model.model_id", manifest.model.model_id.as_str()),
        (
            "model.model_revision",
            manifest.model.model_revision.as_str(),
        ),
        (
            "model.reasoning_effort",
            manifest.model.reasoning_effort.as_str(),
        ),
        ("cache_class", manifest.cache_class.as_str()),
        ("network_policy", manifest.network_policy.as_str()),
        ("tool_policy", manifest.tool_policy.as_str()),
    ] {
        require_text(field, value)?;
    }
    if manifest.network_policy != "disabled_except_loopback" {
        return Err("network_policy must be disabled_except_loopback".to_string());
    }
    match manifest.runtime.executor {
        AgentTokenExecutor::Codex => {
            if manifest.tool_policy != "codex_shell_only" {
                return Err("tool_policy must be codex_shell_only".to_string());
            }
            if manifest.model.provider != "openai" {
                return Err("The codex executor admits only model.provider openai".to_string());
            }
        }
        AgentTokenExecutor::Claude => {
            if manifest.tool_policy != "claude_code_local_tools" {
                return Err(
                    "The claude executor requires tool_policy claude_code_local_tools".to_string(),
                );
            }
            if manifest.model.provider != "anthropic" {
                return Err("The claude executor admits only model.provider anthropic".to_string());
            }
            if manifest.runtime.claude_program.is_none() {
                return Err(
                    "runtime.claude_program is required for the claude executor".to_string()
                );
            }
            if protocol_requires_claude_model_evidence(&manifest.protocol_revision)
                && manifest
                    .runtime
                    .executor_version
                    .as_deref()
                    .is_none_or(|version| version.trim().is_empty())
            {
                return Err("Pinned Claude campaigns require runtime.executor_version".to_string());
            }
            if manifest.claude_model_admission == AgentTokenClaudeModelAdmission::AsShipped
                && !protocol_supports_as_shipped_claude_admission(&manifest.protocol_revision)
            {
                return Err(
                    "claude_model_admission=as_shipped requires protocol .46 or its active successor"
                        .to_string(),
                );
            }
        }
    }
    if manifest.runtime.run_timeout_seconds < 60 {
        return Err("runtime.run_timeout_seconds must be at least 60".to_string());
    }
    for (field, value) in [
        (
            "runtime.executor_version",
            manifest.runtime.executor_version.as_deref(),
        ),
        (
            "runtime.ait_version",
            manifest.runtime.ait_version.as_deref(),
        ),
        (
            "runtime.git_version",
            manifest.runtime.git_version.as_deref(),
        ),
        (
            "runtime.node_version",
            manifest.runtime.node_version.as_deref(),
        ),
        (
            "runtime.browser_version",
            manifest.runtime.browser_version.as_deref(),
        ),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(format!("{field} must not be empty when declared"));
        }
    }
    if manifest.runtime.browser_version.is_some() && manifest.runtime.browser_program.is_none() {
        return Err("runtime.browser_version requires runtime.browser_program".to_string());
    }
    if manifest.runtime.project_doc_max_bytes > 65_536 {
        return Err("runtime.project_doc_max_bytes must not exceed 65536".to_string());
    }
    if manifest.runtime.project_doc_max_bytes > 0
        && manifest.campaign_scope != AgentTokenCampaignScope::Smoke
    {
        return Err(
            "Nonzero runtime.project_doc_max_bytes is admitted only for smoke diagnostics"
                .to_string(),
        );
    }
    if manifest.ait_sprint_mode == AgentTokenAitSprintMode::On {
        if !matches!(
            manifest.campaign_scope,
            AgentTokenCampaignScope::Smoke
                | AgentTokenCampaignScope::Diagnostic
                | AgentTokenCampaignScope::Complete
        ) {
            return Err(
                "AIT sprint-on is admitted only for smoke, diagnostic, or complete campaigns"
                    .to_string(),
            );
        }
        if manifest.accounting_profile != AgentTokenAccountingProfile::SteadyStateTaskCost {
            return Err("AIT sprint-on requires steady_state_task_cost accounting".to_string());
        }
        // A diagnostic campaign deliberately isolates one workload, so the
        // full-matrix requirement applies only to scopes that can be published.
        if manifest.campaign_scope != AgentTokenCampaignScope::Diagnostic
            && workloads != BTreeSet::from(["GD-01", "GD-02", "GD-03", "GD-04", "GD-05"])
        {
            return Err("AIT sprint-on must contain all five workloads".to_string());
        }
        if manifest.runtime.project_doc_max_bytes != 0 {
            return Err("AIT sprint-on cannot mix project-document-loading treatment".to_string());
        }
    }
    if manifest.git_worktree_mode == AgentTokenGitWorktreeMode::CodexAppEquivalentManaged {
        if manifest.protocol_revision != AGENT_TOKEN_PROTOCOL_REVISION
            && manifest.protocol_revision
                != AGENT_TOKEN_MANAGED_WORKTREE_PREFLIGHT_PREDECESSOR_PROTOCOL_REVISION
        {
            return Err(
                "codex_app_equivalent_managed is admitted only by the active prospective protocol or its exact zero-lane preflight predecessor"
                    .to_string(),
            );
        }
        if manifest.runtime.executor != AgentTokenExecutor::Codex {
            return Err("codex_app_equivalent_managed requires the Codex executor".to_string());
        }
        if manifest.accounting_profile != AgentTokenAccountingProfile::SteadyStateTaskCost {
            return Err(
                "codex_app_equivalent_managed requires steady_state_task_cost accounting"
                    .to_string(),
            );
        }
        if manifest.ait_sprint_mode != AgentTokenAitSprintMode::Off {
            return Err(
                "codex_app_equivalent_managed is paired only with the sprint-off AIT treatment"
                    .to_string(),
            );
        }
        if manifest.ait_edit_root_mode != AgentTokenAitEditRootMode::Returned {
            return Err(
                "codex_app_equivalent_managed requires AIT task start without --edit-root"
                    .to_string(),
            );
        }
        if manifest.runtime.project_doc_max_bytes != 0 {
            return Err(
                "codex_app_equivalent_managed cannot mix project-document loading treatment"
                    .to_string(),
            );
        }
    }
    if manifest.functional_replacement_policy != AgentTokenFunctionalReplacementPolicy::None {
        if manifest.protocol_revision != AGENT_TOKEN_PROTOCOL_REVISION
            && !protocol_requires_claude_model_evidence(&manifest.protocol_revision)
        {
            return Err(
                "Functional replacement may be declared only by the active prospective protocol"
                    .to_string(),
            );
        }
        if manifest.campaign_scope != AgentTokenCampaignScope::Complete {
            return Err(
                "Functional replacement is admitted only for complete campaigns; smoke must retain every outcome without replacement"
                    .to_string(),
            );
        }
    }
    for (field, value) in [
        ("runtime.codex_program", &manifest.runtime.codex_program),
        ("runtime.ait_program", &manifest.runtime.ait_program),
        ("runtime.git_program", &manifest.runtime.git_program),
        ("runtime.node_program", &manifest.runtime.node_program),
        (
            "runtime.fixture_manifest",
            &manifest.runtime.fixture_manifest,
        ),
    ] {
        if value.as_os_str().is_empty() {
            return Err(format!("{field} must not be empty"));
        }
    }
    if !manifest.runtime.fixture_manifest.is_file() {
        return Err(format!(
            "runtime.fixture_manifest is unavailable: {}",
            manifest.runtime.fixture_manifest.display()
        ));
    }
    if manifest.accounting_profile == AgentTokenAccountingProfile::FirstUseTotalCost
        && manifest.runtime.ait_first_use_worktree_add_dir.is_none()
    {
        return Err(
            "first_use_total_cost requires runtime.ait_first_use_worktree_add_dir".to_string(),
        );
    }
    if let Some(path) = manifest.runtime.ait_first_use_worktree_add_dir.as_deref() {
        if !path.is_dir() {
            return Err(format!(
                "runtime.ait_first_use_worktree_add_dir is unavailable: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn source_required_attempts_per_cell(manifest: &AgentTokenCampaignManifest) -> usize {
    if manifest.campaign_scope != AgentTokenCampaignScope::Complete
        || manifest.protocol_revision == AGENT_TOKEN_PROTOCOL_REVISION
        || manifest.protocol_revision
            == AGENT_TOKEN_MANAGED_WORKTREE_PREFLIGHT_PREDECESSOR_PROTOCOL_REVISION
        || manifest.protocol_revision == AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION
        || manifest.protocol_revision == AGENT_TOKEN_MODEL_PURITY_PREDECESSOR_PROTOCOL_REVISION
        || manifest.protocol_revision
            == AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION
        || manifest.protocol_revision
            == AGENT_TOKEN_PROMPTED_INSPECTION_PREDECESSOR_PROTOCOL_REVISION
        || manifest.protocol_revision == AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION
        || manifest.protocol_revision == AGENT_TOKEN_200_SESSION_PREDECESSOR_PROTOCOL_REVISION
    {
        return manifest.campaign_scope.minimum_attempts();
    }
    AGENT_TOKEN_PREDECESSOR_COMPLETE_ATTEMPTS_PER_WORKLOAD
}

pub fn build_agent_token_schedule(manifest: &AgentTokenCampaignManifest) -> AgentTokenSchedule {
    let mut entries = Vec::new();
    for attempt in 1..=manifest.attempts_per_cell {
        let mut generator =
            DeterministicRng::new(manifest.seed ^ (attempt as u64).wrapping_mul(0x9E37_79B9));
        let mut workloads = manifest.workload_ids.clone();
        generator.shuffle(&mut workloads);
        let mut block_order = 1_usize;
        for workload_id in workloads {
            let mut pair_modes = manifest.modes.clone();
            generator.shuffle(&mut pair_modes);
            for mode in pair_modes {
                entries.push(AgentTokenScheduleEntry {
                    run_id: format!(
                        "{}-b{attempt:03}-{}-{}",
                        manifest.campaign_id,
                        workload_id.to_ascii_lowercase(),
                        mode.short_name()
                    ),
                    workload_id: workload_id.clone(),
                    mode,
                    attempt,
                    block_index: attempt,
                    randomized_order: block_order,
                });
                block_order += 1;
            }
        }
    }
    AgentTokenSchedule {
        contract: AGENT_TOKEN_SCHEDULE_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        protocol_revision: manifest.protocol_revision.clone(),
        seed: manifest.seed,
        entry_count: entries.len(),
        entries,
    }
}

pub fn import_codex_usage(
    source: &Path,
    run_id: &str,
    workload_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
    model: &AgentTokenModelPin,
) -> Result<NormalizedAgentTokenUsage, String> {
    let source_text = fs::read_to_string(source).map_err(|error| {
        format!(
            "Failed to read Codex JSONL usage source {}: {error}",
            source.display()
        )
    })?;
    let mut input_tokens = 0_u64;
    let mut cached_input_tokens = 0_u64;
    let mut cache_write_input_tokens = 0_u64;
    let mut output_tokens = 0_u64;
    let mut reasoning_tokens = 0_u64;
    let mut cached_available = true;
    let mut cache_write_available = true;
    let mut reasoning_available = true;
    let mut completed_turns = 0_usize;

    for (index, line) in source_text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Codex JSONL {} line {} is invalid: {error}",
                source.display(),
                index + 1
            )
        })?;
        if event.get("type").and_then(serde_json::Value::as_str) != Some("turn.completed") {
            continue;
        }
        let usage = event
            .get("usage")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                format!(
                    "Codex turn.completed line {} has no usage object",
                    index + 1
                )
            })?;
        input_tokens = input_tokens
            .checked_add(required_u64(usage, "input_tokens", index + 1)?)
            .ok_or_else(|| "Codex input token sum overflowed u64".to_string())?;
        output_tokens = output_tokens
            .checked_add(required_u64(usage, "output_tokens", index + 1)?)
            .ok_or_else(|| "Codex output token sum overflowed u64".to_string())?;
        add_optional_u64(
            usage,
            "cached_input_tokens",
            &mut cached_input_tokens,
            &mut cached_available,
            index + 1,
        )?;
        add_optional_u64(
            usage,
            "cache_write_input_tokens",
            &mut cache_write_input_tokens,
            &mut cache_write_available,
            index + 1,
        )?;
        add_optional_u64(
            usage,
            "reasoning_output_tokens",
            &mut reasoning_tokens,
            &mut reasoning_available,
            index + 1,
        )?;
        completed_turns += 1;
    }
    if completed_turns == 0 {
        return Err("Codex JSONL contains no turn.completed usage event".to_string());
    }
    if cached_available && cached_input_tokens > input_tokens {
        return Err("Codex cached_input_tokens exceeds input_tokens".to_string());
    }
    if reasoning_available && reasoning_tokens > output_tokens {
        return Err("Codex reasoning_output_tokens exceeds output_tokens".to_string());
    }
    let provider_total_tokens = input_tokens
        .checked_add(output_tokens)
        .ok_or_else(|| "Codex provider total token sum overflowed u64".to_string())?;
    Ok(NormalizedAgentTokenUsage {
        contract: AGENT_TOKEN_USAGE_CONTRACT.to_string(),
        run_id: run_id.to_string(),
        workload_id: workload_id.to_string(),
        mode,
        accounting_profile: profile,
        model_provider: model.provider.clone(),
        model_id: model.model_id.clone(),
        model_revision: model.model_revision.clone(),
        reasoning_effort: model.reasoning_effort.clone(),
        input_tokens,
        cached_input_tokens: cached_available.then_some(cached_input_tokens),
        cache_write_input_tokens: cache_write_available.then_some(cache_write_input_tokens),
        output_tokens,
        reasoning_tokens: reasoning_available.then_some(reasoning_tokens),
        provider_total_tokens,
        completed_turns,
        usage_provenance: "codex-exec-jsonl:turn.completed".to_string(),
    })
}

#[derive(Clone, Debug)]
pub(crate) struct ImportedClaudeUsage {
    pub usage: NormalizedAgentTokenUsage,
    pub served_models: Vec<AgentTokenServedModelUsage>,
    pub fallback_observed: bool,
    pub provider_refusal: bool,
    pub provider_stop_reason: String,
}

/// Import provider usage and terminal outcome from a Claude Code stream-json
/// transcript. Protocol `.37` treats the stream itself as model-purity
/// evidence: the init model, every assistant message, and the terminal
/// `modelUsage` inventory must all name exactly the pinned model. This rejects
/// prompt-suggestion or fallback model calls instead of silently omitting
/// their tokens. A successful `refusal` stop remains valid, token-accounted
/// model behavior and is returned separately for functional classification.
pub(crate) fn import_claude_usage_with_outcome(
    source: &Path,
    run_id: &str,
    workload_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
    model: &AgentTokenModelPin,
    admission: AgentTokenClaudeModelAdmission,
) -> Result<ImportedClaudeUsage, String> {
    let source_text = fs::read_to_string(source).map_err(|error| {
        format!(
            "Failed to read Claude stream-json usage source {}: {error}",
            source.display()
        )
    })?;
    let mut init_seen = false;
    let mut assistant_event_count = 0_usize;
    let mut fallback_observed = false;
    let mut result_event = None;
    for (index, line) in source_text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Claude stream-json {} line {} is invalid: {error}",
                source.display(),
                index + 1
            )
        })?;
        fallback_observed |= contains_claude_fallback_event(&event);
        match event.get("type").and_then(serde_json::Value::as_str) {
            Some("system")
                if event.get("subtype").and_then(serde_json::Value::as_str) == Some("init") =>
            {
                if init_seen {
                    return Err(
                        "Claude stream-json contains more than one system init event".to_string(),
                    );
                }
                init_seen = true;
                let observed_model = event
                    .get("model")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| "Claude system init carries no model".to_string())?;
                if observed_model != model.model_id {
                    return Err(format!(
                        "Claude system init model differs from the pin: expected {}, got {}",
                        model.model_id, observed_model
                    ));
                }
                let observed_tool_values = event
                    .get("tools")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| "Claude system init carries no tools inventory".to_string())?;
                if observed_tool_values.len() != CLAUDE_MEASURED_TOOL_SURFACE.len() {
                    return Err(format!(
                        "Claude system init tool inventory length differs: expected {}, got {}",
                        CLAUDE_MEASURED_TOOL_SURFACE.len(),
                        observed_tool_values.len()
                    ));
                }
                let observed_tools = observed_tool_values
                    .iter()
                    .map(|value| {
                        value.as_str().map(str::to_string).ok_or_else(|| {
                            "Claude system init tools inventory contains a non-string".to_string()
                        })
                    })
                    .collect::<Result<BTreeSet<_>, _>>()?;
                let expected_tools = CLAUDE_MEASURED_TOOL_SURFACE
                    .iter()
                    .map(|tool| (*tool).to_string())
                    .collect::<BTreeSet<_>>();
                if observed_tools != expected_tools {
                    return Err(format!(
                        "Claude system init tool inventory differs from the declared surface: expected {}, got {}",
                        expected_tools.into_iter().collect::<Vec<_>>().join(", "),
                        observed_tools.into_iter().collect::<Vec<_>>().join(", ")
                    ));
                }
                let mcp_servers = event
                    .get("mcp_servers")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| {
                        "Claude system init carries no MCP-server inventory".to_string()
                    })?;
                if !mcp_servers.is_empty() {
                    return Err("Claude system init activated an MCP server".to_string());
                }
            }
            Some("assistant") => {
                assistant_event_count = assistant_event_count.saturating_add(1);
                let observed_model = event
                    .pointer("/message/model")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| "Claude assistant event carries no message model".to_string())?;
                if observed_model != model.model_id
                    && admission == AgentTokenClaudeModelAdmission::Strict
                {
                    return Err(format!(
                        "Claude assistant model differs from the pin: expected {}, got {}",
                        model.model_id, observed_model
                    ));
                }
            }
            Some("result") => {
                if result_event.is_some() {
                    return Err(
                        "Claude stream-json contains more than one result event".to_string()
                    );
                }
                result_event = Some(event);
            }
            Some("prompt_suggestion") => {
                return Err(
                    "Claude stream-json emitted a prompt suggestion despite the disabled pin"
                        .to_string(),
                );
            }
            _ => {}
        }
    }
    if !init_seen {
        return Err("Claude stream-json contains no system init event".to_string());
    }
    if assistant_event_count == 0 {
        return Err("Claude stream-json contains no assistant event".to_string());
    }
    let Some(result) = result_event else {
        return Err("Claude stream-json contains no result usage event".to_string());
    };
    let subtype = result
        .get("subtype")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Claude result event carries no subtype".to_string())?;
    let is_error = result
        .get("is_error")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| "Claude result event carries no is_error boolean".to_string())?;
    if subtype != "success" || is_error {
        return Err(format!(
            "Claude terminal result is not successful: subtype={subtype}, is_error={is_error}"
        ));
    }
    let terminal_reason = result
        .get("terminal_reason")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Claude result event carries no terminal_reason".to_string())?;
    if terminal_reason != "completed" {
        return Err(format!(
            "Claude terminal_reason is {terminal_reason}, expected completed"
        ));
    }
    let provider_stop_reason = result
        .get("stop_reason")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Claude result event carries no stop_reason".to_string())?
        .to_string();
    let usage = result
        .get("usage")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "Claude result event carries no usage object".to_string())?
        .clone();
    let num_turns = result
        .get("num_turns")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "Claude result event carries no num_turns".to_string())?;
    let direct_input = required_claude_u64(&usage, "input_tokens")?;
    let cache_read = required_claude_u64(&usage, "cache_read_input_tokens")?;
    let cache_write = required_claude_u64(&usage, "cache_creation_input_tokens")?;
    let output_tokens = required_claude_u64(&usage, "output_tokens")?;
    let reasoning_tokens = usage
        .get("output_tokens_details")
        .and_then(|details| details.get("thinking_tokens"))
        .and_then(serde_json::Value::as_u64);
    let input_tokens = direct_input
        .checked_add(cache_read)
        .and_then(|sum| sum.checked_add(cache_write))
        .ok_or_else(|| "Claude input token sum overflowed u64".to_string())?;
    if let Some(reasoning) = reasoning_tokens {
        if reasoning > output_tokens {
            return Err("Claude thinking_tokens exceeds output_tokens".to_string());
        }
    }
    let provider_total_tokens = input_tokens
        .checked_add(output_tokens)
        .ok_or_else(|| "Claude provider total token sum overflowed u64".to_string())?;
    if num_turns == 0 {
        return Err("Claude result event reports zero turns".to_string());
    }
    let model_usage = result
        .get("modelUsage")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "Claude result event carries no modelUsage inventory".to_string())?;
    let mut served_models = model_usage
        .iter()
        .map(|(model_id, value)| parse_claude_served_model_usage(model_id, value))
        .collect::<Result<Vec<_>, _>>()?;
    served_models.sort_by(|left, right| left.model_id.cmp(&right.model_id));
    if admission == AgentTokenClaudeModelAdmission::Strict {
        if model_usage.len() != 1 || !model_usage.contains_key(&model.model_id) {
            return Err(format!(
                "Claude terminal modelUsage must contain only the pinned model {}; observed {}",
                model.model_id,
                model_usage.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    } else if !model_usage.contains_key(&model.model_id) {
        return Err(format!(
            "Claude terminal modelUsage does not contain the pinned model {}; observed {}",
            model.model_id,
            model_usage.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    let pinned_usage = model_usage
        .get(&model.model_id)
        .and_then(serde_json::Value::as_object)
        .expect("the pinned modelUsage entry was checked");
    if admission == AgentTokenClaudeModelAdmission::Strict {
        for (model_usage_field, usage_value) in [
            ("inputTokens", direct_input),
            ("cacheReadInputTokens", cache_read),
            ("cacheCreationInputTokens", cache_write),
            ("outputTokens", output_tokens),
        ] {
            let observed = pinned_usage
                .get(model_usage_field)
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| {
                    format!("Claude modelUsage field {model_usage_field} must be u64")
                })?;
            if observed != usage_value {
                return Err(format!(
                "Claude modelUsage field {model_usage_field} differs from terminal usage: expected {usage_value}, got {observed}"
            ));
            }
        }
        let canonical_model = pinned_usage
            .get("canonicalModel")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "Claude modelUsage carries no canonicalModel".to_string())?;
        if canonical_model != model.model_id {
            return Err(format!(
                "Claude canonical model differs from the pin: expected {}, got {}",
                model.model_id, canonical_model
            ));
        }
    } else {
        for (field, terminal, summed) in [
            (
                "inputTokens",
                direct_input,
                sum_served_model_field(&served_models, |usage| usage.input_tokens)?,
            ),
            (
                "cacheReadInputTokens",
                cache_read,
                sum_served_model_field(&served_models, |usage| usage.cached_input_tokens)?,
            ),
            (
                "cacheCreationInputTokens",
                cache_write,
                sum_served_model_field(&served_models, |usage| usage.cache_write_input_tokens)?,
            ),
            (
                "outputTokens",
                output_tokens,
                sum_served_model_field(&served_models, |usage| usage.output_tokens)?,
            ),
        ] {
            if terminal != summed {
                return Err(format!(
                    "Claude summed modelUsage field {field} differs from terminal usage: expected {terminal}, got {summed}"
                ));
            }
        }
    }
    let usage = NormalizedAgentTokenUsage {
        contract: AGENT_TOKEN_USAGE_CONTRACT.to_string(),
        run_id: run_id.to_string(),
        workload_id: workload_id.to_string(),
        mode,
        accounting_profile: profile,
        model_provider: model.provider.clone(),
        model_id: model.model_id.clone(),
        model_revision: model.model_revision.clone(),
        reasoning_effort: model.reasoning_effort.clone(),
        input_tokens,
        cached_input_tokens: Some(cache_read),
        cache_write_input_tokens: Some(cache_write),
        output_tokens,
        reasoning_tokens,
        provider_total_tokens,
        completed_turns: 1,
        usage_provenance: if admission == AgentTokenClaudeModelAdmission::AsShipped
            && served_models.len() > 1
        {
            "claude-code-stream-json:result+served-model-sum".to_string()
        } else {
            "claude-code-stream-json:result+model-purity".to_string()
        },
    };
    Ok(ImportedClaudeUsage {
        usage,
        served_models,
        fallback_observed,
        provider_refusal: provider_stop_reason == "refusal",
        provider_stop_reason,
    })
}

fn parse_claude_served_model_usage(
    model_id: &str,
    value: &serde_json::Value,
) -> Result<AgentTokenServedModelUsage, String> {
    let usage = value
        .as_object()
        .ok_or_else(|| format!("Claude modelUsage entry {model_id} must be an object"))?;
    let required = |field: &str| {
        usage
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("Claude modelUsage entry {model_id} field {field} must be u64"))
    };
    let input_tokens = required("inputTokens")?;
    let cached_input_tokens = required("cacheReadInputTokens")?;
    let cache_write_input_tokens = required("cacheCreationInputTokens")?;
    let output_tokens = required("outputTokens")?;
    let provider_total_tokens = input_tokens
        .checked_add(cached_input_tokens)
        .and_then(|sum| sum.checked_add(cache_write_input_tokens))
        .and_then(|sum| sum.checked_add(output_tokens))
        .ok_or_else(|| format!("Claude modelUsage entry {model_id} token sum overflowed u64"))?;
    let canonical_model = usage
        .get("canonicalModel")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("Claude modelUsage entry {model_id} carries no canonicalModel"))?;
    Ok(AgentTokenServedModelUsage {
        model_id: model_id.to_string(),
        canonical_model: canonical_model.to_string(),
        input_tokens,
        cached_input_tokens,
        cache_write_input_tokens,
        output_tokens,
        provider_total_tokens,
    })
}

fn sum_served_model_field(
    models: &[AgentTokenServedModelUsage],
    field: impl Fn(&AgentTokenServedModelUsage) -> u64,
) -> Result<u64, String> {
    models.iter().try_fold(0_u64, |sum, model| {
        sum.checked_add(field(model))
            .ok_or_else(|| "Claude modelUsage aggregate overflowed u64".to_string())
    })
}

fn contains_claude_fallback_event(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            object.get("type").and_then(serde_json::Value::as_str) == Some("fallback")
                || object.values().any(contains_claude_fallback_event)
        }
        serde_json::Value::Array(values) => values.iter().any(contains_claude_fallback_event),
        _ => false,
    }
}

pub fn import_claude_usage(
    source: &Path,
    run_id: &str,
    workload_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
    model: &AgentTokenModelPin,
) -> Result<NormalizedAgentTokenUsage, String> {
    import_claude_usage_with_outcome(
        source,
        run_id,
        workload_id,
        mode,
        profile,
        model,
        AgentTokenClaudeModelAdmission::Strict,
    )
    .map(|imported| imported.usage)
}

pub fn extract_and_validate_codex_transcript(
    source: &Path,
    run_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
) -> Result<AgentTokenCommandTranscript, String> {
    extract_and_validate_codex_transcript_with_git_start_proof(source, run_id, mode, profile, false)
}

#[derive(Clone, Copy)]
pub(crate) struct AgentTokenTranscriptWorkflowOptions {
    pub ait_sprint_mode: AgentTokenAitSprintMode,
    pub ait_edit_root_mode: Option<AgentTokenAitEditRootMode>,
    pub git_worktree_mode: AgentTokenGitWorktreeMode,
    pub clean_main_head_proven: bool,
}

pub(crate) fn extract_and_validate_codex_transcript_with_git_start_proof(
    source: &Path,
    run_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
    clean_main_head_proven: bool,
) -> Result<AgentTokenCommandTranscript, String> {
    extract_and_validate_codex_transcript_with_workflow_options(
        source,
        run_id,
        mode,
        profile,
        AgentTokenTranscriptWorkflowOptions {
            ait_sprint_mode: AgentTokenAitSprintMode::Off,
            ait_edit_root_mode: None,
            git_worktree_mode: AgentTokenGitWorktreeMode::AgentManaged,
            clean_main_head_proven,
        },
    )
}

pub(crate) fn extract_and_validate_codex_transcript_with_workflow_options(
    source: &Path,
    run_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
    options: AgentTokenTranscriptWorkflowOptions,
) -> Result<AgentTokenCommandTranscript, String> {
    let source_text = fs::read_to_string(source).map_err(|error| {
        format!(
            "Failed to read Codex JSONL transcript source {}: {error}",
            source.display()
        )
    })?;
    let mut commands = Vec::new();
    let mut seen_items = BTreeSet::new();
    for (index, line) in source_text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Codex JSONL {} line {} is invalid: {error}",
                source.display(),
                index + 1
            )
        })?;
        if event.get("type").and_then(serde_json::Value::as_str) != Some("item.completed") {
            continue;
        }
        let Some(item) = event.get("item").and_then(serde_json::Value::as_object) else {
            continue;
        };
        if item.get("type").and_then(serde_json::Value::as_str) != Some("command_execution") {
            continue;
        }
        let id = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if !id.is_empty() && !seen_items.insert(id.to_string()) {
            continue;
        }
        if let Some(command) = item.get("command").and_then(serde_json::Value::as_str) {
            commands.push(command.to_string());
        }
    }

    validate_agent_token_command_list_with_workflow_options(
        commands, run_id, mode, profile, options,
    )
}

#[cfg(test)]
fn validate_agent_token_command_list(
    commands: Vec<String>,
    run_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
) -> Result<AgentTokenCommandTranscript, String> {
    validate_agent_token_command_list_with_git_start_proof(commands, run_id, mode, profile, false)
}

#[cfg(test)]
fn validate_agent_token_command_list_with_git_start_proof(
    commands: Vec<String>,
    run_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
    clean_main_head_proven: bool,
) -> Result<AgentTokenCommandTranscript, String> {
    validate_agent_token_command_list_with_workflow_options(
        commands,
        run_id,
        mode,
        profile,
        AgentTokenTranscriptWorkflowOptions {
            ait_sprint_mode: AgentTokenAitSprintMode::Off,
            ait_edit_root_mode: None,
            git_worktree_mode: AgentTokenGitWorktreeMode::AgentManaged,
            clean_main_head_proven,
        },
    )
}

fn validate_agent_token_command_list_with_workflow_options(
    commands: Vec<String>,
    run_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
    options: AgentTokenTranscriptWorkflowOptions,
) -> Result<AgentTokenCommandTranscript, String> {
    let AgentTokenTranscriptWorkflowOptions {
        ait_sprint_mode,
        ait_edit_root_mode,
        git_worktree_mode,
        clean_main_head_proven,
    } = options;
    let mut errors = Vec::new();
    let mut observed_required_commands = Vec::new();
    match mode {
        AgentTokenMode::GitLinearSingleSession => {
            let git_invocations = commands
                .iter()
                .flat_map(|command| git_command_invocations(command))
                .filter(|invocation| !invocation_is_help_introspection(invocation))
                .collect::<Vec<_>>();
            // Local read-only inspection is recorded as observed evidence and
            // stays token-accounted, but it is never required: the declared
            // worktree, commit, fast-forward, removal, and branch-deletion
            // lifecycle already proves Git-mode fidelity, and requiring
            // discovery here would charge Git an inspection cost that the AIT
            // branch never pays.
            for (name, subcommand) in [
                ("git status", &["status"][..]),
                ("git diff", &["diff"][..]),
                ("git log", &["log"][..]),
                ("git show", &["show"][..]),
                ("git rev-parse", &["rev-parse"][..]),
            ] {
                if git_invocations
                    .iter()
                    .any(|invocation| git_invocation_has_subcommand(invocation, subcommand))
                {
                    observed_required_commands.push(name.to_string());
                }
            }
            if commands.iter().any(|command| command_invokes_ait(command)) {
                errors.push("Git mode invoked AIT".to_string());
            }
            if git_invocations
                .iter()
                .any(git_invocation_overrides_metadata_context)
                || commands
                    .iter()
                    .any(|command| command_exports_git_metadata_context(command))
            {
                errors.push(GIT_METADATA_CONTEXT_OVERRIDE_ERROR.to_string());
            }
            if git_worktree_mode == AgentTokenGitWorktreeMode::CodexAppEquivalentManaged {
                for invocation in &git_invocations {
                    if git_invocation_is_app_managed_read_only(invocation) {
                        observed_required_commands.push(format!(
                            "read-only git {}",
                            git_subcommand_arguments(invocation).join(" ")
                        ));
                    } else {
                        errors.push(format!(
                            "Codex App-equivalent Git lane invoked host-owned mutation: git {}",
                            git_subcommand_arguments(invocation).join(" ")
                        ));
                    }
                }
                if profile != AgentTokenAccountingProfile::SteadyStateTaskCost {
                    errors.push(
                        "Codex App-equivalent Git lane requires steady-state accounting"
                            .to_string(),
                    );
                }
            } else {
                let commit_count = git_invocations
                    .iter()
                    .filter(|invocation| git_invocation_has_subcommand(invocation, &["commit"]))
                    .count();
                let expected_commit_count =
                    if profile == AgentTokenAccountingProfile::FirstUseTotalCost {
                        2
                    } else {
                        1
                    };
                if commit_count != expected_commit_count {
                    errors.push(format!(
                    "Git mode must execute exactly {expected_commit_count} git commit command(s); observed {commit_count}"
                ));
                } else {
                    observed_required_commands.push("git commit".to_string());
                }

                for (name, subcommand) in [
                    ("git worktree add", &["worktree", "add"][..]),
                    ("git merge", &["merge"][..]),
                    ("git worktree remove", &["worktree", "remove"][..]),
                ] {
                    let count = git_invocations
                        .iter()
                        .filter(|invocation| git_invocation_has_subcommand(invocation, subcommand))
                        .count();
                    if count != 1 {
                        errors.push(format!(
                            "Git mode must execute exactly one {name}; observed {count}"
                        ));
                    } else {
                        observed_required_commands.push(name.to_string());
                    }
                }
                let branch_delete_count = git_invocations
                    .iter()
                    .filter(|invocation| {
                        git_invocation_has_subcommand(invocation, &["branch"])
                            && invocation
                                .arguments
                                .iter()
                                .any(|argument| matches!(argument.as_str(), "-d" | "--delete"))
                    })
                    .count();
                if branch_delete_count != 1 {
                    errors.push(format!(
                    "Git mode must execute exactly one git branch deletion; observed {branch_delete_count}"
                ));
                } else {
                    observed_required_commands.push("git branch --delete".to_string());
                }

                let worktree_add = git_invocations.iter().find(|invocation| {
                    git_invocation_has_subcommand(invocation, &["worktree", "add"])
                });
                match worktree_add.map(classify_git_worktree_add_start) {
                Some(GitWorktreeAddStart::ExplicitMain) => {}
                Some(GitWorktreeAddStart::ImplicitHead)
                    if profile == AgentTokenAccountingProfile::SteadyStateTaskCost
                        && clean_main_head_proven =>
                {
                    observed_required_commands
                        .push("git worktree add from runner-proven clean main HEAD".to_string());
                }
                Some(GitWorktreeAddStart::ImplicitHead) => errors.push(
                    "Git mode omitted the worktree start point without a runner-proven clean main HEAD"
                        .to_string(),
                ),
                None | Some(GitWorktreeAddStart::Invalid) => errors.push(
                    "Git mode did not create the declared benchmark-task linked worktree from main"
                        .to_string(),
                ),
            }
                if !git_invocations.iter().any(|invocation| {
                    git_invocation_has_subcommand(invocation, &["merge"])
                        && invocation_has_option(invocation, "--ff-only")
                        && invocation
                            .arguments
                            .iter()
                            .any(|argument| argument == "benchmark-task")
                }) {
                    errors.push(
                    "Git mode did not fast-forward main to benchmark-task with git merge --ff-only"
                        .to_string(),
                );
                }
                if !git_invocations.iter().any(|invocation| {
                    git_invocation_has_subcommand(invocation, &["worktree", "remove"])
                        && invocation
                            .arguments
                            .iter()
                            .any(|argument| argument.ends_with("git-task-worktree"))
                }) {
                    errors.push(
                        "Git mode did not remove the declared benchmark-owned linked worktree"
                            .to_string(),
                    );
                }
                if !git_invocations.iter().any(|invocation| {
                    git_invocation_has_subcommand(invocation, &["branch"])
                        && invocation
                            .arguments
                            .iter()
                            .any(|argument| matches!(argument.as_str(), "-d" | "--delete"))
                        && invocation
                            .arguments
                            .iter()
                            .any(|argument| argument == "benchmark-task")
                }) {
                    errors.push(
                        "Git mode did not delete the declared benchmark-task branch".to_string(),
                    );
                }
                if git_invocations.iter().any(|invocation| {
                    ["clone", "fetch", "pull", "push", "remote", "ls-remote"]
                        .iter()
                        .any(|subcommand| git_invocation_has_subcommand(invocation, &[*subcommand]))
                }) {
                    errors.push("Git mode invoked a forbidden remote operation".to_string());
                }
                match profile {
                    AgentTokenAccountingProfile::SteadyStateTaskCost => {
                        if git_invocations
                            .iter()
                            .any(|invocation| git_invocation_has_subcommand(invocation, &["init"]))
                        {
                            errors.push(
                                "Steady-state Git mode repeated first-use repository bootstrap"
                                    .to_string(),
                            );
                        }
                    }
                    AgentTokenAccountingProfile::FirstUseTotalCost => {
                        if git_invocations
                            .iter()
                            .any(|invocation| git_invocation_has_subcommand(invocation, &["init"]))
                        {
                            observed_required_commands.push("git init".to_string());
                        } else {
                            errors.push("First-use Git mode did not initialize Git".to_string());
                        }
                        for (required, key) in [
                            ("git config user.name", "user.name"),
                            ("git config user.email", "user.email"),
                        ] {
                            if git_invocations.iter().any(|invocation| {
                                git_invocation_has_subcommand(invocation, &["config"])
                                    && invocation.arguments.iter().any(|argument| argument == key)
                            }) {
                                observed_required_commands.push(required.to_string());
                            } else {
                                errors.push(format!(
                                "First-use Git mode did not pin repository identity: {required}"
                            ));
                            }
                        }
                    }
                }
            }
        }
        AgentTokenMode::AitLinearSingleSession => {
            let ait_invocations = commands
                .iter()
                .flat_map(|command| ait_command_invocations(command))
                .filter(|invocation| {
                    !invocation_is_help_introspection(invocation)
                        && !invocation_is_ait_readonly_inspection(invocation)
                })
                .collect::<Vec<_>>();
            for required in ["ait task start", "ait task finish"] {
                let subcommand = required
                    .strip_prefix("ait ")
                    .expect("required AIT command has the frozen prefix")
                    .split_ascii_whitespace()
                    .collect::<Vec<_>>();
                if ait_invocations
                    .iter()
                    .any(|invocation| invocation_has_subcommand(invocation, &subcommand))
                {
                    observed_required_commands.push(required.to_string());
                } else {
                    errors.push(format!(
                        "AIT mode did not execute required command: {required}"
                    ));
                }
            }
            for (name, subcommand) in [
                ("ait task start", &["task", "start"][..]),
                ("ait task finish", &["task", "finish"][..]),
            ] {
                let count = ait_invocations
                    .iter()
                    .filter(|invocation| invocation_has_subcommand(invocation, subcommand))
                    .count();
                if count != 1 {
                    errors.push(format!(
                        "AIT mode must execute exactly one {name}; observed {count}"
                    ));
                }
            }
            if profile == AgentTokenAccountingProfile::SteadyStateTaskCost {
                if let Some(edit_root_mode) = ait_edit_root_mode {
                    let task_start_supplies_edit_root = ait_invocations.iter().any(|invocation| {
                        invocation_has_subcommand(invocation, &["task", "start"])
                            && invocation_has_option(invocation, "--edit-root")
                    });
                    match (edit_root_mode, task_start_supplies_edit_root) {
                        (AgentTokenAitEditRootMode::Explicit, false) => errors.push(
                            "AIT explicit edit-root treatment omitted --edit-root from task start"
                                .to_string(),
                        ),
                        (AgentTokenAitEditRootMode::Returned, true) => errors.push(
                            "AIT returned edit-root treatment supplied forbidden --edit-root"
                                .to_string(),
                        ),
                        (AgentTokenAitEditRootMode::Explicit, true) => observed_required_commands
                            .push("ait task start with --edit-root".to_string()),
                        (AgentTokenAitEditRootMode::Returned, false) => observed_required_commands
                            .push("ait task start without --edit-root".to_string()),
                    }
                }
            }
            for required in [["task", "start"], ["task", "finish"]] {
                if ait_invocations.iter().any(|invocation| {
                    invocation_has_subcommand(invocation, &required)
                        && !invocation_has_option(invocation, "--local")
                }) {
                    errors.push(format!(
                        "AIT mode omitted --local from ait {} {}",
                        required[0], required[1]
                    ));
                }
            }
            if !ait_invocations.iter().any(|invocation| {
                invocation_has_subcommand(invocation, &["task", "finish"])
                    && invocation_has_option(invocation, "--message")
            }) {
                errors.push(
                    "AIT mode did not use Task finish to create the final local Snapshot"
                        .to_string(),
                );
            }
            if commands
                .iter()
                .any(|command| command_invokes_git_vcs(command))
            {
                errors.push("AIT mode substituted raw Git workflow commands".to_string());
            }
            if commands
                .iter()
                .any(|command| command_invokes_ait_server(command))
            {
                errors.push("AIT mode used forbidden solo-local surface: ait-server".to_string());
            }
            for (forbidden, subcommand) in [
                ("ait push", &["push"][..]),
                ("ait pull", &["pull"][..]),
                ("ait remote", &["remote"][..]),
                ("ait plan", &["plan"][..]),
                ("ait workflow ready", &["workflow", "ready"][..]),
                ("ait queue summary", &["queue", "summary"][..]),
                ("ait task list", &["task", "list"][..]),
                ("ait change list", &["change", "list"][..]),
                ("ait task audit", &["task", "audit"][..]),
            ] {
                if ait_invocations
                    .iter()
                    .any(|invocation| invocation_has_subcommand(invocation, subcommand))
                {
                    errors.push(format!(
                        "AIT mode used forbidden solo-local surface: {forbidden}"
                    ));
                }
            }
            if ait_invocations
                .iter()
                .any(|invocation| invocation_has_option(invocation, "--remote"))
            {
                errors.push("AIT mode used forbidden solo-local surface: --remote".to_string());
            }
            let bound_task_start = ait_invocations.iter().any(|invocation| {
                invocation_has_subcommand(invocation, &["task", "start"])
                    && invocation_has_option(invocation, "--from")
            });
            match ait_sprint_mode {
                AgentTokenAitSprintMode::Off if bound_task_start => errors.push(
                    "AIT sprint-off mode used forbidden solo-local surface: task start --from"
                        .to_string(),
                ),
                AgentTokenAitSprintMode::On if !bound_task_start => errors
                    .push("AIT sprint-on mode did not bind task start with --from".to_string()),
                AgentTokenAitSprintMode::Off | AgentTokenAitSprintMode::On => {}
            }
            for invocation in &ait_invocations {
                let common_lifecycle = invocation_has_subcommand(invocation, &["task", "start"])
                    || invocation_has_subcommand(invocation, &["snapshot", "create"])
                    || invocation_has_subcommand(invocation, &["task", "finish"]);
                let first_use_bootstrap = profile == AgentTokenAccountingProfile::FirstUseTotalCost
                    && (invocation_has_subcommand(invocation, &["init"])
                        || invocation_has_subcommand(invocation, &["config", "set"]));
                if !common_lifecycle && !first_use_bootstrap {
                    errors.push(format!(
                        "AIT mode invoked a command outside the measured lifecycle: ait {}",
                        invocation.arguments.join(" ")
                    ));
                }
            }
            match profile {
                AgentTokenAccountingProfile::SteadyStateTaskCost => {
                    for (forbidden, subcommand) in [
                        ("ait init", &["init"][..]),
                        ("ait config set", &["config", "set"][..]),
                    ] {
                        if ait_invocations
                            .iter()
                            .any(|invocation| invocation_has_subcommand(invocation, subcommand))
                        {
                            errors.push(format!(
                                "Steady-state AIT mode repeated first-use bootstrap: {forbidden}"
                            ));
                        }
                    }
                }
                AgentTokenAccountingProfile::FirstUseTotalCost => {
                    for (required, subcommand) in [
                        ("ait init", &["init"][..]),
                        ("ait config set", &["config", "set"][..]),
                    ] {
                        if ait_invocations
                            .iter()
                            .any(|invocation| invocation_has_subcommand(invocation, subcommand))
                        {
                            observed_required_commands.push(required.to_string());
                        } else {
                            errors.push(format!(
                                "First-use AIT mode did not execute bootstrap command: {required}"
                            ));
                        }
                    }
                    let snapshot_count = ait_invocations
                        .iter()
                        .filter(|invocation| {
                            invocation_has_subcommand(invocation, &["snapshot", "create"])
                        })
                        .count();
                    if snapshot_count < 2 {
                        errors.push(
                            "First-use AIT mode did not create separate baseline and task Snapshots"
                                .to_string(),
                        );
                    }
                }
            }
        }
    }
    if commands
        .iter()
        .any(|command| command.contains("npm test") || command.contains("scripts/self-test.mjs"))
    {
        observed_required_commands.push("project-local validation".to_string());
    } else {
        errors.push("Candidate did not execute project-local validation".to_string());
    }

    Ok(AgentTokenCommandTranscript {
        contract: AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
        run_id: run_id.to_string(),
        mode,
        accounting_profile: profile,
        command_count: commands.len(),
        commands,
        valid: errors.is_empty(),
        errors,
        observed_required_commands,
    })
}

/// Extract the measured shell commands from a Claude Code stream-json
/// transcript and validate them against the same mode contract the codex
/// executor uses. Commands are the `Bash` tool_use inputs; every other
/// tool (Read, Grep, Glob, Edit, Write) is a non-shell surface and is
/// validated separately by the patch-attempt metrics.
pub fn extract_and_validate_claude_transcript(
    source: &Path,
    run_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
) -> Result<AgentTokenCommandTranscript, String> {
    extract_and_validate_claude_transcript_with_git_start_proof(
        source, run_id, mode, profile, false,
    )
}

pub(crate) fn extract_and_validate_claude_transcript_with_git_start_proof(
    source: &Path,
    run_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
    clean_main_head_proven: bool,
) -> Result<AgentTokenCommandTranscript, String> {
    extract_and_validate_claude_transcript_with_workflow_options(
        source,
        run_id,
        mode,
        profile,
        AgentTokenTranscriptWorkflowOptions {
            ait_sprint_mode: AgentTokenAitSprintMode::Off,
            ait_edit_root_mode: None,
            git_worktree_mode: AgentTokenGitWorktreeMode::AgentManaged,
            clean_main_head_proven,
        },
    )
}

pub(crate) fn extract_and_validate_claude_transcript_with_workflow_options(
    source: &Path,
    run_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
    options: AgentTokenTranscriptWorkflowOptions,
) -> Result<AgentTokenCommandTranscript, String> {
    let source_text = fs::read_to_string(source).map_err(|error| {
        format!(
            "Failed to read Claude stream-json transcript source {}: {error}",
            source.display()
        )
    })?;
    let mut commands = Vec::new();
    let mut seen_items = BTreeSet::new();
    let mut out_of_surface_tools = BTreeSet::new();
    for (index, line) in source_text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Claude stream-json {} line {} is invalid: {error}",
                source.display(),
                index + 1
            )
        })?;
        if event.get("type").and_then(serde_json::Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(content) = event
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_use") {
                continue;
            }
            let tool_name = block
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if !CLAUDE_MEASURED_TOOL_SURFACE.contains(&tool_name) {
                out_of_surface_tools.insert(if tool_name.is_empty() {
                    "<unnamed>".to_string()
                } else {
                    tool_name.to_string()
                });
                continue;
            }
            if tool_name != "Bash" {
                continue;
            }
            let id = block
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if !id.is_empty() && !seen_items.insert(id.to_string()) {
                continue;
            }
            if let Some(command) = block
                .get("input")
                .and_then(|input| input.get("command"))
                .and_then(serde_json::Value::as_str)
            {
                commands.push(command.to_string());
            }
        }
    }
    let mut transcript = validate_agent_token_command_list_with_workflow_options(
        commands, run_id, mode, profile, options,
    )?;
    if !out_of_surface_tools.is_empty() {
        transcript.valid = false;
        transcript.errors.push(format!(
            "Claude session used tools outside the declared measured surface: {}",
            out_of_surface_tools
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(transcript)
}

pub fn extract_agent_token_secondary_metrics(
    source: &Path,
    transcript: &AgentTokenCommandTranscript,
) -> Result<AgentTokenSecondaryMetrics, String> {
    let source_text = fs::read_to_string(source).map_err(|error| {
        format!(
            "Failed to read Codex JSONL metrics source {}: {error}",
            source.display()
        )
    })?;
    let mut metrics = AgentTokenSecondaryMetrics::default();
    for (index, line) in source_text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Codex JSONL {} line {} is invalid: {error}",
                source.display(),
                index + 1
            )
        })?;
        if event.get("type").and_then(serde_json::Value::as_str) == Some("turn.completed") {
            metrics.agent_turns += 1;
            metrics.model_calls += 1;
        }
        if event.get("type").and_then(serde_json::Value::as_str) != Some("item.completed") {
            continue;
        }
        let Some(item) = event.get("item").and_then(serde_json::Value::as_object) else {
            continue;
        };
        match item.get("type").and_then(serde_json::Value::as_str) {
            Some("command_execution") => {
                if let Some(output) = item
                    .get("aggregated_output")
                    .and_then(serde_json::Value::as_str)
                {
                    metrics.tool_output_bytes = metrics
                        .tool_output_bytes
                        .saturating_add(output.len() as u64);
                }
            }
            Some("file_change") => metrics.file_change_items += 1,
            _ => {}
        }
    }
    apply_shared_command_statistics(&mut metrics, transcript);
    Ok(metrics)
}

/// Shell-command-derived statistics shared by every executor: family
/// grouping, validation/query/help/read classification, and repeated-query
/// accounting operate on the validated transcript alone.
fn apply_shared_command_statistics(
    metrics: &mut AgentTokenSecondaryMetrics,
    transcript: &AgentTokenCommandTranscript,
) {
    metrics.command_tool_calls = transcript.commands.len();
    let mut query_families = BTreeSet::new();
    for command in &transcript.commands {
        let family = command_family(command);
        *metrics
            .tool_calls_by_family
            .entry(family.to_string())
            .or_default() += 1;
        if command.contains("npm test")
            || command.contains("release-check")
            || command.contains("cargo test")
            || command.contains("node scripts/self-test.mjs")
        {
            metrics.project_validation_calls += 1;
        }
        if [
            " status", " diff", " log", " show", " blame", " list", " audit",
        ]
        .iter()
        .any(|needle| command.contains(needle))
        {
            metrics.repository_query_calls += 1;
            query_families.insert(family);
        }
        if command.contains(" --help") || command.contains(" help") {
            metrics.help_calls += 1;
        }
        if command.contains(" rg ")
            || command.contains("rg --")
            || command.contains(" sed ")
            || command.contains(" cat ")
            || command.contains(" head ")
            || command.contains(" tail ")
        {
            metrics.file_read_or_search_calls += 1;
        }
    }
    metrics.repeated_repository_query_calls = metrics
        .repository_query_calls
        .saturating_sub(query_families.len());
}

/// Claude-executor counterpart of the secondary metrics extractor. Tool
/// output bytes come from tool_result content; completed file changes are
/// successful Edit/Write tool uses; rejected patch attempts are Edit/Write
/// tool uses whose tool_result reports is_error. Shell-command-derived
/// statistics reuse the shared classification below.
pub fn extract_agent_token_claude_secondary_metrics(
    source: &Path,
    transcript: &AgentTokenCommandTranscript,
) -> Result<AgentTokenSecondaryMetrics, String> {
    let source_text = fs::read_to_string(source).map_err(|error| {
        format!(
            "Failed to read Claude stream-json metrics source {}: {error}",
            source.display()
        )
    })?;
    let mut metrics = AgentTokenSecondaryMetrics::default();
    let mut edit_tool_ids = BTreeSet::new();
    for (index, line) in source_text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Claude stream-json {} line {} is invalid: {error}",
                source.display(),
                index + 1
            )
        })?;
        match event.get("type").and_then(serde_json::Value::as_str) {
            Some("assistant") => {
                metrics.model_calls += 1;
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
                        .unwrap_or("");
                    if matches!(name, "Edit" | "Write") {
                        metrics.apply_patch_attempts += 1;
                        if let Some(id) = block.get("id").and_then(serde_json::Value::as_str) {
                            edit_tool_ids.insert(id.to_string());
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
                    if let Some(text) = block.get("content").and_then(serde_json::Value::as_str) {
                        metrics.tool_output_bytes =
                            metrics.tool_output_bytes.saturating_add(text.len() as u64);
                    } else if let Some(parts) =
                        block.get("content").and_then(serde_json::Value::as_array)
                    {
                        for part in parts {
                            if let Some(text) = part.get("text").and_then(serde_json::Value::as_str)
                            {
                                metrics.tool_output_bytes =
                                    metrics.tool_output_bytes.saturating_add(text.len() as u64);
                            }
                        }
                    }
                    let is_error = block
                        .get("is_error")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    let for_edit = block
                        .get("tool_use_id")
                        .and_then(serde_json::Value::as_str)
                        .map(|id| edit_tool_ids.contains(id))
                        .unwrap_or(false);
                    if for_edit {
                        if is_error {
                            metrics.apply_patch_rejected_attempts += 1;
                        } else {
                            metrics.file_change_items += 1;
                        }
                    }
                }
            }
            Some("result") => {
                if let Some(turns) = event.get("num_turns").and_then(serde_json::Value::as_u64) {
                    metrics.agent_turns = usize::try_from(turns).unwrap_or(usize::MAX);
                }
            }
            _ => {}
        }
    }
    apply_shared_command_statistics(&mut metrics, transcript);
    Ok(metrics)
}

pub fn build_agent_token_report(
    manifest: &AgentTokenCampaignManifest,
    schedule: &AgentTokenSchedule,
    runs: &[AgentTokenRunSummary],
) -> Result<AgentTokenReport, String> {
    let mut blockers = Vec::new();
    let expected_ids = schedule
        .entries
        .iter()
        .map(|entry| entry.run_id.as_str())
        .collect::<BTreeSet<_>>();
    let observed_ids = runs
        .iter()
        .map(|run| run.run_id.as_str())
        .collect::<BTreeSet<_>>();
    if expected_ids != observed_ids {
        blockers.push("Observed run IDs do not exactly match the frozen schedule".to_string());
    }
    let invalid_run_count = runs.iter().filter(|run| !run.valid_attempt).count();
    if invalid_run_count > 0 {
        blockers.push(format!("{invalid_run_count} run(s) are protocol-invalid"));
    }

    let mut by_group = BTreeMap::<(String, AgentTokenMode), Vec<&AgentTokenRunSummary>>::new();
    for run in runs {
        by_group
            .entry((run.workload_id.clone(), run.mode))
            .or_default()
            .push(run);
    }
    let mut groups = Vec::new();
    for ((workload_id, mode), grouped) in by_group {
        let attempted_count = grouped.len();
        let valid = grouped
            .iter()
            .copied()
            .filter(|run| run.valid_attempt)
            .collect::<Vec<_>>();
        let accepted_count = valid.iter().filter(|run| run.accepted_equivalent).count();
        let usage = valid
            .iter()
            .filter_map(|run| run.usage.as_ref())
            .collect::<Vec<_>>();
        let tokens = usage
            .iter()
            .map(|usage| usage.provider_total_tokens)
            .collect::<Vec<_>>();
        let total_valid_attempt_tokens = tokens.iter().copied().sum::<u64>();
        let effective_tokens_per_accepted_task = (accepted_count > 0)
            .then_some(total_valid_attempt_tokens as f64 / accepted_count as f64);
        let distribution_seed =
            manifest.seed ^ stable_text_seed(&workload_id) ^ stable_text_seed(mode.as_str());
        let valid_attempt_token_distribution =
            summarize_optional_u64(&tokens, manifest.bootstrap_resamples, distribution_seed)?;
        let input_token_distribution = summarize_optional_u64(
            &usage
                .iter()
                .map(|usage| usage.input_tokens)
                .collect::<Vec<_>>(),
            manifest.bootstrap_resamples,
            distribution_seed ^ 0x10,
        )?;
        let cached = usage
            .iter()
            .filter_map(|usage| usage.cached_input_tokens)
            .collect::<Vec<_>>();
        let cached_input_token_distribution = if cached.len() == usage.len() {
            summarize_optional_u64(
                &cached,
                manifest.bootstrap_resamples,
                distribution_seed ^ 0x20,
            )?
        } else {
            None
        };
        let output_token_distribution = summarize_optional_u64(
            &usage
                .iter()
                .map(|usage| usage.output_tokens)
                .collect::<Vec<_>>(),
            manifest.bootstrap_resamples,
            distribution_seed ^ 0x30,
        )?;
        let reasoning = usage
            .iter()
            .filter_map(|usage| usage.reasoning_tokens)
            .collect::<Vec<_>>();
        let reasoning_token_distribution = if reasoning.len() == usage.len() {
            summarize_optional_u64(
                &reasoning,
                manifest.bootstrap_resamples,
                distribution_seed ^ 0x40,
            )?
        } else {
            None
        };
        let elapsed_ms_distribution = summarize_optional_u64(
            &valid.iter().map(|run| run.elapsed_ms).collect::<Vec<_>>(),
            manifest.bootstrap_resamples,
            distribution_seed ^ 0x50,
        )?;
        let completed_file_change_item_distribution = summarize_optional_u64(
            &valid
                .iter()
                .map(|run| run.secondary_metrics.file_change_items as u64)
                .collect::<Vec<_>>(),
            manifest.bootstrap_resamples,
            distribution_seed ^ 0x60,
        )?;
        let rejected_apply_patch_attempt_distribution = summarize_optional_u64(
            &valid
                .iter()
                .map(|run| run.secondary_metrics.apply_patch_rejected_attempts as u64)
                .collect::<Vec<_>>(),
            manifest.bootstrap_resamples,
            distribution_seed ^ 0x70,
        )?;
        let apply_patch_attempt_distribution = summarize_optional_u64(
            &valid
                .iter()
                .map(|run| run.secondary_metrics.apply_patch_attempts as u64)
                .collect::<Vec<_>>(),
            manifest.bootstrap_resamples,
            distribution_seed ^ 0x80,
        )?;
        let host_provisioning = valid
            .iter()
            .filter_map(|run| run.secondary_metrics.host_worktree_provisioning_elapsed_ms)
            .collect::<Vec<_>>();
        let host_worktree_provisioning_elapsed_ms_distribution =
            if host_provisioning.len() == valid.len() && !valid.is_empty() {
                summarize_optional_u64(
                    &host_provisioning,
                    manifest.bootstrap_resamples,
                    distribution_seed ^ 0x90,
                )?
            } else {
                None
            };
        let host_closeout = valid
            .iter()
            .filter_map(|run| run.secondary_metrics.host_worktree_closeout_elapsed_ms)
            .collect::<Vec<_>>();
        let host_worktree_closeout_elapsed_ms_distribution =
            if host_closeout.len() == valid.len() && !valid.is_empty() {
                summarize_optional_u64(
                    &host_closeout,
                    manifest.bootstrap_resamples,
                    distribution_seed ^ 0xA0,
                )?
            } else {
                None
            };
        groups.push(AgentTokenGroupReport {
            workload_id,
            mode: mode.as_str().to_string(),
            attempted_count,
            valid_count: valid.len(),
            accepted_count,
            invalid_count: attempted_count - valid.len(),
            acceptance_rate: if valid.is_empty() {
                0.0
            } else {
                accepted_count as f64 / valid.len() as f64
            },
            total_valid_attempt_tokens,
            effective_tokens_per_accepted_task,
            valid_attempt_token_distribution,
            input_token_distribution,
            cached_input_token_distribution,
            output_token_distribution,
            reasoning_token_distribution,
            elapsed_ms_distribution,
            completed_file_change_item_distribution,
            rejected_apply_patch_attempt_distribution,
            apply_patch_attempt_distribution,
            host_worktree_provisioning_elapsed_ms_distribution,
            host_worktree_closeout_elapsed_ms_distribution,
        });
    }
    groups.sort_by(|left, right| {
        (&left.workload_id, &left.mode).cmp(&(&right.workload_id, &right.mode))
    });

    let mut comparisons = Vec::new();
    let mut comparison_bootstrap_samples = Vec::new();
    let mut acceptance_rate_deficit_exceeded = false;
    for workload_id in &manifest.workload_ids {
        let git_candidates = runs
            .iter()
            .filter(|run| {
                run.workload_id == *workload_id
                    && run.mode == AgentTokenMode::GitLinearSingleSession
                    && run.valid_attempt
                    && run.usage.is_some()
            })
            .collect::<Vec<_>>();
        let ait_candidates = runs
            .iter()
            .filter(|run| {
                run.workload_id == *workload_id
                    && run.mode == AgentTokenMode::AitLinearSingleSession
                    && run.valid_attempt
                    && run.usage.is_some()
            })
            .collect::<Vec<_>>();
        let (git_runs, ait_runs) = paired_runs(&git_candidates, &ait_candidates);
        let git_effective = failure_adjusted_effective_tokens(&git_runs);
        let ait_effective = failure_adjusted_effective_tokens(&ait_runs);
        let token_savings_percent = relative_reduction(git_effective, ait_effective);
        let git_effective_elapsed_ms =
            failure_adjusted_effective_measure(&git_runs, |run| Some(run.elapsed_ms));
        let ait_effective_elapsed_ms =
            failure_adjusted_effective_measure(&ait_runs, |run| Some(run.elapsed_ms));
        let elapsed_savings_percent =
            relative_reduction(git_effective_elapsed_ms, ait_effective_elapsed_ms);
        let git_effective_completed_file_change_items =
            failure_adjusted_effective_measure(&git_runs, |run| {
                Some(run.secondary_metrics.file_change_items as u64)
            });
        let ait_effective_completed_file_change_items =
            failure_adjusted_effective_measure(&ait_runs, |run| {
                Some(run.secondary_metrics.file_change_items as u64)
            });
        let completed_file_change_reduction_percent = relative_reduction(
            git_effective_completed_file_change_items,
            ait_effective_completed_file_change_items,
        );
        let git_effective_rejected_apply_patch_attempts =
            failure_adjusted_effective_measure(&git_runs, |run| {
                Some(run.secondary_metrics.apply_patch_rejected_attempts as u64)
            });
        let ait_effective_rejected_apply_patch_attempts =
            failure_adjusted_effective_measure(&ait_runs, |run| {
                Some(run.secondary_metrics.apply_patch_rejected_attempts as u64)
            });
        let rejected_apply_patch_reduction_percent = relative_reduction(
            git_effective_rejected_apply_patch_attempts,
            ait_effective_rejected_apply_patch_attempts,
        );
        let git_effective_apply_patch_attempts =
            failure_adjusted_effective_measure(&git_runs, |run| {
                Some(run.secondary_metrics.apply_patch_attempts as u64)
            });
        let ait_effective_apply_patch_attempts =
            failure_adjusted_effective_measure(&ait_runs, |run| {
                Some(run.secondary_metrics.apply_patch_attempts as u64)
            });
        let apply_patch_attempt_reduction_percent = relative_reduction(
            git_effective_apply_patch_attempts,
            ait_effective_apply_patch_attempts,
        );
        let git_acceptance_rate = acceptance_rate(&git_runs);
        let ait_acceptance_rate = acceptance_rate(&ait_runs);
        let acceptance_rate_deficit_percentage_points =
            acceptance_rate_percentage_points(&git_runs)
                - acceptance_rate_percentage_points(&ait_runs);
        acceptance_rate_deficit_exceeded |=
            acceptance_rate_deficit_exceeds_five_percentage_points(&git_runs, &ait_runs);
        let bootstrap = bootstrap_failure_adjusted_savings(
            &git_runs,
            &ait_runs,
            manifest.bootstrap_resamples,
            manifest.seed ^ stable_text_seed(workload_id),
        );
        let paired_valid_attempt_count = paired_attempt_count(&git_runs, &ait_runs);
        let token_savings_bootstrap_ci95 = bootstrap.as_ref().map(|samples| {
            [
                quantile_r7_local(samples, 0.025),
                quantile_r7_local(samples, 0.975),
            ]
        });
        comparison_bootstrap_samples.push(bootstrap);
        comparisons.push(AgentTokenModeComparison {
            workload_id: workload_id.clone(),
            git_effective_tokens: git_effective,
            ait_effective_tokens: ait_effective,
            token_savings_percent,
            token_savings_bootstrap_ci95,
            paired_valid_attempt_count,
            git_effective_elapsed_ms,
            ait_effective_elapsed_ms,
            elapsed_savings_percent,
            git_effective_completed_file_change_items,
            ait_effective_completed_file_change_items,
            completed_file_change_reduction_percent,
            git_effective_rejected_apply_patch_attempts,
            ait_effective_rejected_apply_patch_attempts,
            rejected_apply_patch_reduction_percent,
            git_effective_apply_patch_attempts,
            ait_effective_apply_patch_attempts,
            apply_patch_attempt_reduction_percent,
            git_acceptance_rate,
            ait_acceptance_rate,
            acceptance_rate_deficit_percentage_points,
        });
    }

    let savings = comparisons
        .iter()
        .filter_map(|comparison| comparison.token_savings_percent)
        .collect::<Vec<_>>();
    let aggregate_median_token_savings_percent =
        (savings.len() == manifest.workload_ids.len()).then(|| quantile_r7_local(&savings, 0.5));
    let aggregate_bootstrap = aggregate_bootstrap_medians(&comparison_bootstrap_samples);
    let aggregate_token_savings_bootstrap_ci95 = aggregate_bootstrap.as_ref().map(|samples| {
        [
            quantile_r7_local(samples, 0.025),
            quantile_r7_local(samples, 0.975),
        ]
    });
    let aggregate_median_elapsed_savings_percent =
        complete_comparison_median(&comparisons, manifest.workload_ids.len(), |comparison| {
            comparison.elapsed_savings_percent
        });
    let aggregate_median_completed_file_change_reduction_percent =
        complete_comparison_median(&comparisons, manifest.workload_ids.len(), |comparison| {
            comparison.completed_file_change_reduction_percent
        });
    let aggregate_median_rejected_apply_patch_reduction_percent =
        complete_comparison_median(&comparisons, manifest.workload_ids.len(), |comparison| {
            comparison.rejected_apply_patch_reduction_percent
        });
    let aggregate_median_apply_patch_attempt_reduction_percent =
        complete_comparison_median(&comparisons, manifest.workload_ids.len(), |comparison| {
            comparison.apply_patch_attempt_reduction_percent
        });

    let complete_group_counts = groups.len() == manifest.workload_ids.len() * manifest.modes.len()
        && groups.iter().all(|group| {
            group.attempted_count == manifest.attempts_per_cell
                && group.valid_count == manifest.attempts_per_cell
                && group.accepted_count == manifest.attempts_per_cell
        });
    let complete_pair_counts = comparisons.len() == manifest.workload_ids.len()
        && comparisons
            .iter()
            .all(|comparison| comparison.paired_valid_attempt_count == manifest.attempts_per_cell);
    if !complete_group_counts || !complete_pair_counts {
        blockers.push("The required accepted paired schedule is incomplete".to_string());
    }
    let source_scheduled_run_count = manifest
        .workload_ids
        .len()
        .saturating_mul(manifest.modes.len())
        .saturating_mul(manifest.attempts_per_cell);
    if manifest.campaign_scope == AgentTokenCampaignScope::Complete
        && (schedule.entry_count != source_scheduled_run_count
            || schedule.entries.len() != source_scheduled_run_count
            || runs.len() != source_scheduled_run_count)
    {
        blockers.push(format!(
            "Complete campaign evidence must contain exactly {source_scheduled_run_count} scheduled runs"
        ));
    }
    if comparisons
        .iter()
        .any(|comparison| comparison.token_savings_percent.is_none())
    {
        blockers
            .push("At least one workload lacks an accepted effective-token comparison".to_string());
    }
    if acceptance_rate_deficit_exceeded {
        blockers.push("AIT acceptance-rate deficit exceeds five percentage points".to_string());
    }
    if aggregate_median_token_savings_percent.is_none() {
        blockers.push("Aggregate median savings is unavailable".to_string());
    }
    if aggregate_token_savings_bootstrap_ci95.is_none_or(|interval| interval[0] <= 0.0) {
        blockers
            .push("Aggregate token-savings bootstrap 95% lower bound is not positive".to_string());
    }
    if manifest.runtime.project_doc_max_bytes > 0 {
        blockers.push(
            "Project-document-loading diagnostic evidence is not publication eligible".to_string(),
        );
    }
    if manifest.ait_sprint_mode == AgentTokenAitSprintMode::On
        && manifest.campaign_scope == AgentTokenCampaignScope::Smoke
    {
        blockers.push("AIT sprint-on diagnostic evidence is not publication eligible".to_string());
    }
    let positive_statistics = aggregate_median_token_savings_percent
        .is_some_and(|saving| saving > 0.0)
        && aggregate_token_savings_bootstrap_ci95.is_some_and(|interval| interval[0] > 0.0);

    let current_complete_group_counts = manifest.workload_ids
        == ["GD-01", "GD-02", "GD-03", "GD-04", "GD-05"]
        && groups.len() == 10
        && groups.iter().all(|group| {
            group.attempted_count == AGENT_TOKEN_COMPLETE_ATTEMPTS_PER_WORKLOAD
                && group.valid_count == AGENT_TOKEN_COMPLETE_ATTEMPTS_PER_WORKLOAD
                && group.accepted_count == AGENT_TOKEN_COMPLETE_ATTEMPTS_PER_WORKLOAD
        });
    let current_complete_pair_counts = comparisons.len() == 5
        && comparisons.iter().all(|comparison| {
            comparison.paired_valid_attempt_count == AGENT_TOKEN_COMPLETE_ATTEMPTS_PER_WORKLOAD
        });
    let current_complete_schedule = schedule.entry_count == AGENT_TOKEN_COMPLETE_SCHEDULED_RUNS
        && schedule.entries.len() == AGENT_TOKEN_COMPLETE_SCHEDULED_RUNS
        && runs.len() == AGENT_TOKEN_COMPLETE_SCHEDULED_RUNS;
    let mut current_policy_blockers = blockers.clone();
    if !current_complete_group_counts || !current_complete_pair_counts || !current_complete_schedule
    {
        current_policy_blockers.push(format!(
            "Current policy {AGENT_TOKEN_PROTOCOL_REVISION} requires the exact five-workload, two-mode, twenty-pair, 200-session matrix"
        ));
    }
    let current_policy_criteria_met = current_policy_blockers.is_empty() && positive_statistics;

    match manifest.campaign_scope {
        AgentTokenCampaignScope::Smoke => {
            blockers.push("Smoke evidence is not publication eligible".to_string());
        }
        AgentTokenCampaignScope::Diagnostic => {
            blockers.push("diagnostic evidence is never claim eligible".to_string());
        }
        AgentTokenCampaignScope::Pilot => {
            blockers.push("pilot evidence is never claim eligible".to_string());
        }
        AgentTokenCampaignScope::Qualification | AgentTokenCampaignScope::Complete => {}
    }
    let source_protocol_claim_eligible = matches!(
        manifest.campaign_scope,
        AgentTokenCampaignScope::Qualification | AgentTokenCampaignScope::Complete
    ) && blockers.is_empty()
        && positive_statistics;
    let claim_eligible = source_protocol_claim_eligible;

    let mut limitations = manifest.limitations.clone();
    if manifest.runtime.project_doc_max_bytes > 0 {
        limitations.push(format!(
            "Diagnostic project-document loading was enabled symmetrically with project_doc_max_bytes={}; results are screening evidence only.",
            manifest.runtime.project_doc_max_bytes
        ));
    }
    if manifest.ait_sprint_mode == AgentTokenAitSprintMode::On {
        let scope_label = if manifest.campaign_scope == AgentTokenCampaignScope::Smoke {
            "smoke result"
        } else {
            "complete result"
        };
        limitations.push(
            format!(
                "The AIT lane measured sprint-card authoring, task binding, and automatic checklist closeout; the Git lane retained its frozen linked-worktree treatment. This {scope_label} is reported separately from sprint-off evidence."
            ),
        );
    }
    if manifest.git_worktree_mode == AgentTokenGitWorktreeMode::CodexAppEquivalentManaged {
        limitations.push(
            "The Git lane reproduces the observable Codex App managed-worktree boundary with runner-owned detached-worktree provisioning and closeout. Codex Desktop's private worktree IPC was not invoked, so this result measures App-equivalent model context rather than Desktop UI implementation latency."
                .to_string(),
        );
    }
    if manifest.functional_replacement_policy
        == AgentTokenFunctionalReplacementPolicy::FirstValidUnacceptedLaneOnce
    {
        limitations.push(
            "The complete campaign prospectively permits exactly one same-mode replacement for the first protocol-valid unaccepted lane in frozen schedule order. The raw failure remains disclosed; smoke, token direction, and a failed replacement cannot authorize another execution."
                .to_string(),
        );
    }

    Ok(AgentTokenReport {
        contract: AGENT_TOKEN_REPORT_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        protocol_revision: manifest.protocol_revision.clone(),
        campaign_scope: manifest.campaign_scope.as_str().to_string(),
        accounting_profile: manifest.accounting_profile.as_str().to_string(),
        ait_sprint_mode: manifest.ait_sprint_mode,
        ait_edit_root_mode: manifest.ait_edit_root_mode,
        git_worktree_mode: manifest.git_worktree_mode,
        functional_replacement_policy: manifest.functional_replacement_policy,
        model: manifest.model.clone(),
        cache_class: manifest.cache_class.clone(),
        network_policy: manifest.network_policy.clone(),
        project_doc_max_bytes: manifest.runtime.project_doc_max_bytes,
        pair_admission_policy: AGENT_TOKEN_PAIR_ADMISSION_POLICY.to_string(),
        generated_at: Utc::now().to_rfc3339(),
        scheduled_run_count: schedule.entry_count,
        observed_run_count: runs.len(),
        executed_evidence_run_count: runs.len(),
        statistically_excluded_run_count: 0,
        invalid_run_count,
        served_models: Vec::new(),
        mixed_model_run_count: 0,
        fallback_observed_run_count: 0,
        groups,
        comparisons,
        aggregate_median_token_savings_percent,
        aggregate_token_savings_bootstrap_ci95,
        aggregate_median_elapsed_savings_percent,
        aggregate_median_completed_file_change_reduction_percent,
        aggregate_median_rejected_apply_patch_reduction_percent,
        aggregate_median_apply_patch_attempt_reduction_percent,
        source_protocol_claim_eligible,
        current_policy_revision: AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
        current_policy_evaluation_mode: if manifest.protocol_revision
            == AGENT_TOKEN_PROTOCOL_REVISION
        {
            "prospective".to_string()
        } else {
            "retrospective".to_string()
        },
        current_policy_criteria_met,
        current_policy_blockers,
        source_protocol_blockers: blockers.clone(),
        replacement_policy_revision: None,
        statistical_replacements: Vec::new(),
        infrastructure_recovery_policy_revision: None,
        infrastructure_pair_recoveries: Vec::new(),
        host_shutdown_recovery_policy_revision: None,
        host_shutdown_pair_recoveries: Vec::new(),
        recovered_spawn_policy_revision: None,
        recovered_spawn_adjudications: Vec::new(),
        claim_eligible,
        blockers,
        limitations,
    })
}

pub fn load_agent_token_report(path: &Path) -> Result<AgentTokenReport, String> {
    let mut report = read_json::<AgentTokenReport>(path, "agent-token report")?;
    if report.current_policy_revision.is_empty() {
        report.source_protocol_claim_eligible = false;
        report.current_policy_revision = AGENT_TOKEN_PROTOCOL_REVISION.to_string();
        report.current_policy_evaluation_mode = "not_evaluated".to_string();
        report.current_policy_criteria_met = false;
        report.current_policy_blockers.push(
            "Legacy report lacks dual-policy provenance; regenerate it from the immutable source campaign"
                .to_string(),
        );
        report.claim_eligible = false;
        if !report.blockers.iter().any(|blocker| {
            blocker
                == "Legacy report lacks dual-policy provenance; regenerate it from the immutable source campaign"
        }) {
            report.blockers.push(
                "Legacy report lacks dual-policy provenance; regenerate it from the immutable source campaign"
                    .to_string(),
            );
        }
    }
    Ok(report)
}

pub fn compare_agent_token_reports(
    baseline: &AgentTokenReport,
    candidate: &AgentTokenReport,
) -> AgentTokenCrossCampaignReport {
    let mut blockers = Vec::new();
    if baseline.protocol_revision != candidate.protocol_revision {
        blockers.push("protocol revision differs".to_string());
    }
    if baseline.campaign_scope != candidate.campaign_scope {
        blockers.push("campaign scope differs".to_string());
    }
    if baseline.accounting_profile != candidate.accounting_profile {
        blockers.push("accounting profile differs".to_string());
    }
    if baseline.ait_sprint_mode != candidate.ait_sprint_mode {
        blockers.push("AIT sprint mode differs".to_string());
    }
    if baseline.ait_edit_root_mode != candidate.ait_edit_root_mode {
        blockers.push("AIT edit-root delivery differs".to_string());
    }
    if baseline.git_worktree_mode != candidate.git_worktree_mode {
        blockers.push("Git worktree ownership differs".to_string());
    }
    if baseline.functional_replacement_policy != candidate.functional_replacement_policy {
        blockers.push("functional replacement policy differs".to_string());
    }
    if baseline.model != candidate.model {
        blockers.push("model pin differs".to_string());
    }
    if baseline.cache_class != candidate.cache_class {
        blockers.push("cache class differs".to_string());
    }
    if baseline.network_policy != candidate.network_policy {
        blockers.push("network policy differs".to_string());
    }
    if baseline.project_doc_max_bytes != candidate.project_doc_max_bytes {
        blockers.push("project-document loading differs".to_string());
    }
    let mut keys = baseline
        .groups
        .iter()
        .map(|group| (group.workload_id.clone(), group.mode.clone()))
        .collect::<BTreeSet<_>>();
    keys.extend(
        candidate
            .groups
            .iter()
            .map(|group| (group.workload_id.clone(), group.mode.clone())),
    );
    let mut deltas = Vec::new();
    for (workload_id, mode) in keys {
        let baseline_group = baseline
            .groups
            .iter()
            .find(|group| group.workload_id == workload_id && group.mode == mode);
        let candidate_group = candidate
            .groups
            .iter()
            .find(|group| group.workload_id == workload_id && group.mode == mode);
        if baseline_group.is_none() || candidate_group.is_none() {
            blockers.push(format!("group inventory differs for {workload_id}/{mode}"));
        }
        let baseline_effective_tokens =
            baseline_group.and_then(|group| group.effective_tokens_per_accepted_task);
        let candidate_effective_tokens =
            candidate_group.and_then(|group| group.effective_tokens_per_accepted_task);
        let effective_token_delta_percent =
            match (baseline_effective_tokens, candidate_effective_tokens) {
                (Some(baseline_value), Some(candidate_value)) if baseline_value > 0.0 => {
                    Some(100.0 * (candidate_value / baseline_value - 1.0))
                }
                _ => None,
            };
        let baseline_acceptance_rate = baseline_group.map_or(0.0, |group| group.acceptance_rate);
        let candidate_acceptance_rate = candidate_group.map_or(0.0, |group| group.acceptance_rate);
        deltas.push(AgentTokenCrossCampaignDelta {
            workload_id,
            mode,
            baseline_effective_tokens,
            candidate_effective_tokens,
            effective_token_delta_percent,
            baseline_acceptance_rate,
            candidate_acceptance_rate,
            acceptance_rate_delta_percentage_points: 100.0
                * (candidate_acceptance_rate - baseline_acceptance_rate),
        });
    }
    AgentTokenCrossCampaignReport {
        contract: "ait-agent-token-cross-campaign-comparison/v1".to_string(),
        baseline_campaign_id: baseline.campaign_id.clone(),
        candidate_campaign_id: candidate.campaign_id.clone(),
        comparable: blockers.is_empty(),
        deltas,
        blockers,
    }
}

pub fn load_agent_token_schedule(path: &Path) -> Result<AgentTokenSchedule, String> {
    read_json(path, "agent-token schedule")
}

pub fn load_agent_token_run_summaries(
    campaign_dir: &Path,
) -> Result<Vec<AgentTokenRunSummary>, String> {
    load_agent_token_run_summaries_with_allowed_missing(campaign_dir, None)
}

pub(crate) fn load_agent_token_run_summaries_with_allowed_missing(
    campaign_dir: &Path,
    allowed_missing_run_id: Option<&str>,
) -> Result<Vec<AgentTokenRunSummary>, String> {
    let mut runs = load_agent_token_raw_run_summaries_with_allowed_missing(
        campaign_dir,
        allowed_missing_run_id,
    )?;
    let manifest_path = campaign_dir.join("campaign-manifest.json");
    let mut adjudication_manifest = None;
    for run in &mut runs {
        let path = campaign_dir
            .join("adjudications")
            .join(format!("{}.json", run.run_id));
        if !path.exists() {
            continue;
        }
        let manifest = match adjudication_manifest.as_ref() {
            Some(manifest) => manifest,
            None => {
                adjudication_manifest = Some(read_json::<AgentTokenCampaignManifest>(
                    &manifest_path,
                    "agent-token campaign manifest",
                )?);
                adjudication_manifest
                    .as_ref()
                    .expect("adjudication manifest was assigned")
            }
        };
        let adjudication =
            read_json::<AgentTokenRunAdjudication>(&path, "agent-token run adjudication")?;
        if adjudication.source_protocol_revision != manifest.protocol_revision {
            return Err(format!(
                "Run {} adjudication source protocol {} differs from campaign protocol {}",
                run.run_id, adjudication.source_protocol_revision, manifest.protocol_revision
            ));
        }
        *run = validate_agent_token_run_adjudication(campaign_dir, run, adjudication)?;
    }
    Ok(runs)
}

pub fn load_agent_token_raw_run_summaries(
    campaign_dir: &Path,
) -> Result<Vec<AgentTokenRunSummary>, String> {
    load_agent_token_raw_run_summaries_with_allowed_missing(campaign_dir, None)
}

pub(crate) fn load_agent_token_raw_run_summaries_with_allowed_missing(
    campaign_dir: &Path,
    allowed_missing_run_id: Option<&str>,
) -> Result<Vec<AgentTokenRunSummary>, String> {
    let runs_dir = campaign_dir.join("runs");
    let mut entries = fs::read_dir(&runs_dir)
        .map_err(|error| {
            format!(
                "Failed to read campaign runs {}: {error}",
                runs_dir.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Failed to enumerate campaign runs: {error}"))?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut runs = Vec::new();
    for entry in entries {
        if !entry
            .file_type()
            .map_err(|error| format!("Failed to inspect run entry: {error}"))?
            .is_dir()
        {
            continue;
        }
        let summary_path = entry.path().join("run-summary.json");
        if !summary_path.exists()
            && allowed_missing_run_id.is_some_and(|allowed| entry.file_name() == allowed)
        {
            continue;
        }
        runs.push(read_json(&summary_path, "agent-token run summary")?);
    }
    Ok(runs)
}

pub fn build_agent_token_run_adjudication(
    campaign_dir: &Path,
    source: &AgentTokenRunSummary,
    source_protocol_revision: &str,
) -> Result<AgentTokenRunAdjudication, String> {
    if source_protocol_revision == AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION {
        return build_legacy_agent_token_run_adjudication(
            campaign_dir,
            source,
            source_protocol_revision,
        );
    }
    if source_protocol_revision == AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION
        && source.campaign_id == AGENT_TOKEN_RECOVERED_SPAWN_CAMPAIGN_ID
        && source.run_id == AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID
    {
        return build_recovered_spawn_run_adjudication(
            campaign_dir,
            source,
            source_protocol_revision,
        );
    }
    Err(format!(
        "Run {} is not admitted by a narrow agent-token adjudication contract",
        source.run_id
    ))
}

fn build_legacy_agent_token_run_adjudication(
    campaign_dir: &Path,
    source: &AgentTokenRunSummary,
    source_protocol_revision: &str,
) -> Result<AgentTokenRunAdjudication, String> {
    if source.invalid_reasons.len() != 1
        || source.invalid_reasons.first().map(String::as_str)
            != Some(GIT_METADATA_CONTEXT_OVERRIDE_ERROR)
        || source.transcript.errors.len() != 1
        || source.transcript.errors.first().map(String::as_str)
            != Some(GIT_METADATA_CONTEXT_OVERRIDE_ERROR)
        || source.transcript.valid
        || source.mode != AgentTokenMode::GitLinearSingleSession
    {
        return Err(format!(
            "Run {} is not the narrow legacy Git metadata-query false positive",
            source.run_id
        ));
    }
    let run_dir = campaign_dir.join("runs").join(&source.run_id);
    let raw_events = run_dir.join("private/codex-events.raw.jsonl");
    let transcript = extract_and_validate_codex_transcript(
        &raw_events,
        &source.run_id,
        source.mode,
        source.accounting_profile,
    )?;
    if !transcript.valid {
        return Err(format!(
            "Run {} remains transcript-invalid under adjudicator {}: {}",
            source.run_id,
            AGENT_TOKEN_LEGACY_ADJUDICATOR_REVISION,
            transcript.errors.join("; ")
        ));
    }
    if transcript.contract != source.transcript.contract
        || transcript.run_id != source.transcript.run_id
        || transcript.mode != source.transcript.mode
        || transcript.accounting_profile != source.transcript.accounting_profile
        || transcript.command_count != source.transcript.command_count
        || transcript.commands != source.transcript.commands
        || transcript.observed_required_commands != source.transcript.observed_required_commands
    {
        return Err(format!(
            "Run {} transcript adjudication changed evidence outside valid/errors",
            source.run_id
        ));
    }
    let mut effective_summary = source.clone();
    effective_summary.transcript = transcript;
    effective_summary.invalid_reasons.clear();
    effective_summary.valid_attempt = true;
    effective_summary.accepted_equivalent = !effective_summary.codex_timed_out
        && effective_summary.codex_exit_code == Some(0)
        && effective_summary.infrastructure_failure.is_none()
        && effective_summary.usage.is_some()
        && effective_summary.evaluator_accepted
        && effective_summary.browser.status == "passed"
        && effective_summary.workflow_closed;
    if !effective_summary.accepted_equivalent || !effective_summary.failure_reasons.is_empty() {
        return Err(format!(
            "Run {} passed transcript adjudication but does not have unchanged accepted functional evidence",
            source.run_id
        ));
    }
    let source_path = run_dir.join("run-summary.json");
    let source_bytes = fs::read(&source_path).map_err(|error| {
        format!(
            "Failed to read source run summary {}: {error}",
            source_path.display()
        )
    })?;
    Ok(AgentTokenRunAdjudication {
        contract: AGENT_TOKEN_RUN_ADJUDICATION_CONTRACT.to_string(),
        campaign_id: source.campaign_id.clone(),
        run_id: source.run_id.clone(),
        source_protocol_revision: source_protocol_revision.to_string(),
        adjudicator_revision: AGENT_TOKEN_LEGACY_ADJUDICATOR_REVISION.to_string(),
        source_run_summary_sha256: sha256_digest(&source_bytes),
        reason: "read-only git rev-parse --git-dir was previously conflated with a global Git metadata override".to_string(),
        effective_summary,
    })
}

fn build_recovered_spawn_run_adjudication(
    campaign_dir: &Path,
    source: &AgentTokenRunSummary,
    source_protocol_revision: &str,
) -> Result<AgentTokenRunAdjudication, String> {
    let expected_invalid_reason =
        "candidate infrastructure unavailable: codex_tool_process_spawn_failure";
    if source.campaign_id != AGENT_TOKEN_RECOVERED_SPAWN_CAMPAIGN_ID
        || source.run_id != AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID
        || source_protocol_revision != AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION
        || source.mode != AgentTokenMode::GitLinearSingleSession
        || source.workload_id != "GD-03"
        || source.attempt != 17
        || source.block_index != 17
        || source.infrastructure_failure.as_deref() != Some("codex_tool_process_spawn_failure")
        || source.invalid_reasons.as_slice() != [expected_invalid_reason]
        || source.valid_attempt
        || source.accepted_equivalent
        || source.codex_exit_code != Some(0)
        || source.codex_timed_out
        || source.usage.as_ref().is_none_or(|usage| {
            usage.completed_turns == 0
                || usage.provider_total_tokens == 0
                || usage.run_id != source.run_id
                || usage.workload_id != source.workload_id
                || usage.mode != source.mode
                || usage.accounting_profile != source.accounting_profile
        })
        || !source.transcript.valid
        || !source.transcript.errors.is_empty()
        || source.transcript.command_count == 0
        || source.evaluator_exit_code.is_none()
    {
        return Err(format!(
            "Run {} is not the exact recovered-spawn correction candidate",
            source.run_id
        ));
    }

    let run_dir = campaign_dir.join("runs").join(&source.run_id);
    let source_path = run_dir.join("run-summary.json");
    let source_bytes = fs::read(&source_path).map_err(|error| {
        format!(
            "Failed to read source run summary {}: {error}",
            source_path.display()
        )
    })?;
    let source_digest = sha256_digest(&source_bytes);
    if source_digest != AGENT_TOKEN_RECOVERED_SPAWN_SOURCE_SUMMARY_SHA256 {
        return Err(format!(
            "Run {} source summary digest differs from the exact recovered-spawn authorization",
            source.run_id
        ));
    }

    let raw_events_path = run_dir.join("private/codex-events.raw.jsonl");
    let stderr_path = run_dir.join("private/codex.stderr.txt");
    let raw_events = fs::read_to_string(&raw_events_path).map_err(|error| {
        format!(
            "Failed to read recovered-spawn event evidence {}: {error}",
            raw_events_path.display()
        )
    })?;
    let stderr = fs::read_to_string(&stderr_path).map_err(|error| {
        format!(
            "Failed to read recovered-spawn stderr evidence {}: {error}",
            stderr_path.display()
        )
    })?;
    let spawn_signature = (stderr.contains("codex_core::tools::router: error=exec_command failed")
        && stderr.contains("CreateProcess"))
        || raw_events.lines().any(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .is_some_and(|event| {
                    event
                        .pointer("/item/type")
                        .and_then(serde_json::Value::as_str)
                        == Some("command_execution")
                        && event
                            .pointer("/item/exit_code")
                            .and_then(serde_json::Value::as_i64)
                            == Some(-1)
                })
        });
    let successful_command = raw_events.lines().any(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .is_some_and(|event| {
                event
                    .pointer("/item/type")
                    .and_then(serde_json::Value::as_str)
                    == Some("command_execution")
                    && event
                        .pointer("/item/exit_code")
                        .and_then(serde_json::Value::as_i64)
                        == Some(0)
            })
    });
    let terminal_turn = raw_events.lines().any(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|event| {
                event
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .as_deref()
            == Some("turn.completed")
    });
    if !spawn_signature || !successful_command || !terminal_turn {
        return Err(format!(
            "Run {} lacks the exact spawn, recovery, or terminal event evidence",
            source.run_id
        ));
    }

    let mut effective_summary = source.clone();
    effective_summary.infrastructure_failure = None;
    effective_summary.invalid_reasons.clear();
    effective_summary.valid_attempt = true;
    effective_summary.accepted_equivalent = effective_summary.evaluator_accepted
        && effective_summary.browser.status == "passed"
        && effective_summary.workflow_closed
        && effective_summary.failure_reasons.is_empty();

    Ok(AgentTokenRunAdjudication {
        contract: AGENT_TOKEN_RUN_ADJUDICATION_CONTRACT.to_string(),
        campaign_id: source.campaign_id.clone(),
        run_id: source.run_id.clone(),
        source_protocol_revision: source_protocol_revision.to_string(),
        adjudicator_revision: AGENT_TOKEN_ADJUDICATOR_REVISION.to_string(),
        source_run_summary_sha256: source_digest,
        reason: AGENT_TOKEN_RECOVERED_SPAWN_REASON.to_string(),
        effective_summary,
    })
}

fn validate_agent_token_run_adjudication(
    campaign_dir: &Path,
    source: &AgentTokenRunSummary,
    adjudication: AgentTokenRunAdjudication,
) -> Result<AgentTokenRunSummary, String> {
    let expected = build_agent_token_run_adjudication(
        campaign_dir,
        source,
        &adjudication.source_protocol_revision,
    )?;
    let observed = serde_json::to_value(&adjudication)
        .map_err(|error| format!("Failed to normalize run adjudication: {error}"))?;
    let expected = serde_json::to_value(&expected)
        .map_err(|error| format!("Failed to normalize expected run adjudication: {error}"))?;
    if observed != expected {
        return Err(format!(
            "Run {} adjudication differs from its source evidence and current narrow correction",
            source.run_id
        ));
    }
    Ok(adjudication.effective_summary)
}

pub fn write_json_new(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Failed to encode JSON for {}: {error}", path.display()))?;
    bytes.push(b'\n');
    write_bytes_new(path, &bytes)
}

pub fn write_text_new(path: &Path, value: &str) -> Result<(), String> {
    write_bytes_new(path, value.as_bytes())
}

pub fn render_agent_token_report_markdown(report: &AgentTokenReport) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "# Agent Token Benchmark: {}\n\n",
        report.campaign_id
    ));
    output.push_str(&format!(
        "- Source protocol: `{}`\n",
        report.protocol_revision
    ));
    output.push_str(&format!(
        "- Source campaign scope: `{}`\n",
        report.campaign_scope
    ));
    output.push_str(&format!("- Accounting: `{}`\n", report.accounting_profile));
    output.push_str(&format!(
        "- AIT edit-root delivery: `{}`\n",
        report.ait_edit_root_mode.as_str()
    ));
    output.push_str(&format!(
        "- Git worktree ownership: `{}`\n",
        report.git_worktree_mode.as_str()
    ));
    output.push_str(&format!(
        "- Source-protocol claim eligible: `{}`\n",
        report.source_protocol_claim_eligible
    ));
    output.push_str(&format!(
        "- Current policy: `{}` (`{}` evaluation)\n",
        report.current_policy_revision, report.current_policy_evaluation_mode
    ));
    output.push_str(&format!(
        "- Current-policy criteria met: `{}`\n",
        report.current_policy_criteria_met
    ));
    output.push_str(&format!(
        "- Effective `claim_eligible`: `{}`\n",
        report.claim_eligible
    ));
    output.push_str(&format!(
        "- Aggregate median token savings: `{}`\n",
        report
            .aggregate_median_token_savings_percent
            .map(|value| format!("{value:.2}%"))
            .unwrap_or_else(|| "n/a".to_string())
    ));
    output.push_str(&format!(
        "- Aggregate median elapsed savings: `{}`\n",
        display_optional_percent(report.aggregate_median_elapsed_savings_percent)
    ));
    output.push_str(&format!(
        "- Aggregate median completed file-change reduction: `{}`\n",
        display_optional_percent(report.aggregate_median_completed_file_change_reduction_percent,)
    ));
    output.push_str(&format!(
        "- Aggregate median total patch-attempt reduction: `{}`\n",
        display_optional_percent(report.aggregate_median_apply_patch_attempt_reduction_percent)
    ));
    output.push_str(&format!(
        "- Runs: `{}/{}` observed\n",
        report.observed_run_count, report.scheduled_run_count
    ));
    if report.executed_evidence_run_count != report.observed_run_count
        || report.statistically_excluded_run_count > 0
    {
        output.push_str(&format!(
            "- Executed evidence sessions: `{}`\n- Statistically excluded sessions: `{}`\n",
            report.executed_evidence_run_count, report.statistically_excluded_run_count
        ));
    }
    output.push_str(&format!(
        "- Pair admission: `{}`\n\n",
        report.pair_admission_policy
    ));
    if !report.served_models.is_empty() {
        output.push_str("## Served Model Composition\n\n");
        output.push_str(&format!(
            "- Mixed-model runs: `{}`\n- Explicit fallback-observed runs: `{}`\n\n",
            report.mixed_model_run_count, report.fallback_observed_run_count
        ));
        output.push_str("| Served model | Canonical model | Runs | Input | Cache read | Cache write | Output | Total |\n");
        output.push_str("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |\n");
        for model in &report.served_models {
            output.push_str(&format!(
                "| `{}` | `{}` | {} | {} | {} | {} | {} | {} |\n",
                model.model_id,
                model.canonical_model,
                model.run_count,
                model.input_tokens,
                model.cached_input_tokens,
                model.cache_write_input_tokens,
                model.output_tokens,
                model.provider_total_tokens,
            ));
        }
        output.push('\n');
    }
    output.push_str("## Workload Results\n\n");
    output.push_str(
        "| Workload | Mode | Attempts | Accepted | Acceptance | Effective tokens | p50 tokens |\n",
    );
    output.push_str("| --- | --- | ---: | ---: | ---: | ---: | ---: |\n");
    for group in &report.groups {
        output.push_str(&format!(
            "| {} | `{}` | {} | {} | {:.1}% | {} | {} |\n",
            group.workload_id,
            group.mode,
            group.attempted_count,
            group.accepted_count,
            group.acceptance_rate * 100.0,
            group
                .effective_tokens_per_accepted_task
                .map(|value| format!("{value:.1}"))
                .unwrap_or_else(|| "n/a".to_string()),
            group
                .valid_attempt_token_distribution
                .as_ref()
                .map(|distribution| format!("{:.1}", distribution.p50))
                .unwrap_or_else(|| "n/a".to_string()),
        ));
    }
    output.push_str("\n## AIT vs Git Tokens\n\n");
    output.push_str("| Workload | Git effective | AIT effective | Savings | Savings CI95 | Valid pairs | AIT acceptance deficit |\n");
    output.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for comparison in &report.comparisons {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {:.1} pp |\n",
            comparison.workload_id,
            display_optional(comparison.git_effective_tokens),
            display_optional(comparison.ait_effective_tokens),
            display_optional_percent(comparison.token_savings_percent),
            comparison
                .token_savings_bootstrap_ci95
                .map(|value| format!("[{:.2}%, {:.2}%]", value[0], value[1]))
                .unwrap_or_else(|| "n/a".to_string()),
            comparison.paired_valid_attempt_count,
            comparison.acceptance_rate_deficit_percentage_points,
        ));
    }
    if report.git_worktree_mode == AgentTokenGitWorktreeMode::CodexAppEquivalentManaged {
        output.push_str("\n## Host-Managed Worktree Overhead\n\n");
        output.push_str(
            "These runner-owned times occur outside the terminal model event and are not added to model elapsed or token accounting.\n\n",
        );
        output.push_str("| Workload | Provision p50 ms | Closeout p50 ms | Samples |\n");
        output.push_str("| --- | ---: | ---: | ---: |\n");
        for group in report
            .groups
            .iter()
            .filter(|group| group.mode == AgentTokenMode::GitLinearSingleSession.as_str())
        {
            let provisioning = group
                .host_worktree_provisioning_elapsed_ms_distribution
                .as_ref();
            let closeout = group
                .host_worktree_closeout_elapsed_ms_distribution
                .as_ref();
            output.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                group.workload_id,
                provisioning
                    .map(|distribution| format!("{:.1}", distribution.p50))
                    .unwrap_or_else(|| "n/a".to_string()),
                closeout
                    .map(|distribution| format!("{:.1}", distribution.p50))
                    .unwrap_or_else(|| "n/a".to_string()),
                provisioning
                    .map(|distribution| distribution.sample_count.to_string())
                    .unwrap_or_else(|| "0".to_string()),
            ));
        }
    }
    output.push_str("\n## Workflow Efficiency\n\n");
    output.push_str("| Workload | Elapsed Git/AIT ms | Elapsed saving | File changes Git/AIT | File-change reduction | Rejected patches Git/AIT | Rejected reduction | Total patches Git/AIT | Total-patch reduction |\n");
    output.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for comparison in &report.comparisons {
        output.push_str(&format!(
            "| {} | {}/{} | {} | {}/{} | {} | {}/{} | {} | {}/{} | {} |\n",
            comparison.workload_id,
            display_optional(comparison.git_effective_elapsed_ms),
            display_optional(comparison.ait_effective_elapsed_ms),
            display_optional_percent(comparison.elapsed_savings_percent),
            display_optional(comparison.git_effective_completed_file_change_items),
            display_optional(comparison.ait_effective_completed_file_change_items),
            display_optional_percent(comparison.completed_file_change_reduction_percent),
            display_optional(comparison.git_effective_rejected_apply_patch_attempts),
            display_optional(comparison.ait_effective_rejected_apply_patch_attempts),
            display_optional_percent(comparison.rejected_apply_patch_reduction_percent),
            display_optional(comparison.git_effective_apply_patch_attempts),
            display_optional(comparison.ait_effective_apply_patch_attempts),
            display_optional_percent(comparison.apply_patch_attempt_reduction_percent),
        ));
    }
    if !report.statistical_replacements.is_empty() {
        output.push_str("\n## Statistical Replacements\n\n");
        for replacement in &report.statistical_replacements {
            output.push_str(&format!(
                "- `{}` was retained but excluded from effective statistics and replaced by `{}`: {}\n",
                replacement.source_run_id, replacement.replacement_run_id, replacement.reason
            ));
        }
    }
    if !report.infrastructure_pair_recoveries.is_empty() {
        output.push_str("\n## Infrastructure Pair Recovery\n\n");
        for recovery in &report.infrastructure_pair_recoveries {
            output.push_str(&format!(
                "- `{}` attempt {} excluded observed source lane(s) `{}` and admitted the one-time full-pair replacement `{}`: {}\n",
                recovery.workload_id,
                recovery.attempt,
                recovery.observed_source_run_ids.join("`, `"),
                recovery.replacement_run_ids.join("`, `"),
                recovery.reason
            ));
        }
    }
    if !report.host_shutdown_pair_recoveries.is_empty() {
        output.push_str("\n## Host-Shutdown Pair Recovery\n\n");
        for recovery in &report.host_shutdown_pair_recoveries {
            output.push_str(&format!(
                "- `{}` attempt {} retained and excluded the incomplete lane `{}` ({} inventoried artifacts; event `{}`), then admitted the one-time full-pair replacement `{}`: {}\n",
                recovery.workload_id,
                recovery.attempt,
                recovery.interrupted_run_id,
                recovery.interrupted_artifact_count,
                recovery.interrupted_event_sha256,
                recovery.replacement_run_ids.join("`, `"),
                recovery.reason
            ));
        }
    }
    if !report.recovered_spawn_adjudications.is_empty() {
        output.push_str("\n## Recovered Spawn Adjudication\n\n");
        for adjudication in &report.recovered_spawn_adjudications {
            output.push_str(&format!(
                "- `{}` retains raw failure `{}` at summary SHA-256 `{}` and is admitted by adjudicator `{}`: {}\n",
                adjudication.run_id,
                adjudication.source_infrastructure_failure,
                adjudication.source_run_summary_sha256,
                adjudication.adjudicator_revision,
                adjudication.reason
            ));
        }
    }
    if !report.limitations.is_empty() {
        output.push_str("\n## Limitations\n\n");
        for limitation in &report.limitations {
            output.push_str(&format!("- {limitation}\n"));
        }
    }
    if !report.blockers.is_empty() {
        output.push_str("\n## Claim Blockers\n\n");
        for blocker in &report.blockers {
            output.push_str(&format!("- {blocker}\n"));
        }
    }
    if (report.replacement_policy_revision.is_some()
        || report.infrastructure_recovery_policy_revision.is_some()
        || report.host_shutdown_recovery_policy_revision.is_some()
        || report.recovered_spawn_policy_revision.is_some())
        && !report.source_protocol_blockers.is_empty()
    {
        output.push_str("\n## Frozen Source-Protocol Blockers\n\n");
        for blocker in &report.source_protocol_blockers {
            output.push_str(&format!("- {blocker}\n"));
        }
    }
    if !report.current_policy_blockers.is_empty() {
        output.push_str("\n## Current-Policy Criteria Blockers\n\n");
        for blocker in &report.current_policy_blockers {
            output.push_str(&format!("- {blocker}\n"));
        }
    }
    output
}

fn resolve_campaign_paths(
    manifest_path: &Path,
    manifest: &mut AgentTokenCampaignManifest,
) -> Result<(), String> {
    let parent = manifest_path.parent().ok_or_else(|| {
        format!(
            "Agent-token campaign manifest has no parent: {}",
            manifest_path.display()
        )
    })?;
    if manifest.runtime.fixture_manifest.is_relative() {
        manifest.runtime.fixture_manifest = parent.join(&manifest.runtime.fixture_manifest);
    }
    if let Some(path) = manifest.runtime.ait_first_use_worktree_add_dir.as_mut() {
        if path.is_relative() {
            *path = parent.join(&*path);
        }
    }
    Ok(())
}

fn required_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    line: usize,
) -> Result<u64, String> {
    object
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("Codex usage line {line} field {field} must be u64"))
}

fn required_claude_u64(
    map: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<u64, String> {
    map.get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("Claude terminal usage field {field} must be u64"))
}

fn add_optional_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    total: &mut u64,
    available: &mut bool,
    line: usize,
) -> Result<(), String> {
    match object.get(field) {
        Some(value) => {
            let value = value
                .as_u64()
                .ok_or_else(|| format!("Codex usage line {line} field {field} must be u64"))?;
            *total = total
                .checked_add(value)
                .ok_or_else(|| format!("Codex usage field {field} overflowed u64"))?;
        }
        None => *available = false,
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CommandInvocation {
    executable: String,
    arguments: Vec<String>,
    environment_assignments: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitWorktreeAddStart {
    ExplicitMain,
    ImplicitHead,
    Invalid,
}

fn classify_git_worktree_add_start(invocation: &CommandInvocation) -> GitWorktreeAddStart {
    let arguments = git_subcommand_arguments(invocation);
    if arguments.first().map(String::as_str) != Some("worktree")
        || arguments.get(1).map(String::as_str) != Some("add")
    {
        return GitWorktreeAddStart::Invalid;
    }
    let lifecycle = &arguments[2..];
    match lifecycle {
        [branch_flag, branch, path]
            if branch_flag == "-b"
                && branch == "benchmark-task"
                && path.ends_with("git-task-worktree") =>
        {
            GitWorktreeAddStart::ImplicitHead
        }
        [branch_flag, branch, path, start]
            if branch_flag == "-b"
                && branch == "benchmark-task"
                && path.ends_with("git-task-worktree")
                && start == "main" =>
        {
            GitWorktreeAddStart::ExplicitMain
        }
        _ => GitWorktreeAddStart::Invalid,
    }
}

/// A repository-CLI invocation that only prints usage help: it carries
/// `--help`/`-h` anywhere (clap and git print help and exit without
/// executing the action) or opens with the `help` subcommand. Such
/// invocations are informational: token cost stays measured while
/// lifecycle-count and surface checks skip them.
/// A read-only AIT inspection invocation (`ait status`, `ait diff`,
/// `ait blame`) carrying no remote authority. Informational on the same
/// terms as help introspection.
fn invocation_is_ait_readonly_inspection(invocation: &CommandInvocation) -> bool {
    if invocation_has_option(invocation, "--remote") {
        return false;
    }
    AIT_INFORMATIONAL_INSPECTION_SUBCOMMANDS
        .iter()
        .any(|subcommand| invocation_has_subcommand(invocation, subcommand))
}

fn invocation_is_help_introspection(invocation: &CommandInvocation) -> bool {
    invocation
        .arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
        || invocation.arguments.first().map(String::as_str) == Some("help")
}

fn finish_shell_word(current: &mut String, words: &mut Vec<String>) {
    if !current.is_empty() {
        words.push(std::mem::take(current));
    }
}

fn finish_shell_command(words: &mut Vec<String>, commands: &mut Vec<Vec<String>>) {
    if !words.is_empty() {
        commands.push(std::mem::take(words));
    }
}

fn parse_shell_commands(source: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        match quote {
            Some('\'') => {
                if character == '\'' {
                    quote = None;
                } else {
                    current.push(character);
                }
            }
            Some('"') => {
                if character == '"' {
                    quote = None;
                } else if character == '\\'
                    && characters
                        .peek()
                        .is_some_and(|next| matches!(next, '"' | '\\'))
                {
                    current.push(characters.next().expect("peeked escaped character"));
                } else {
                    current.push(character);
                }
            }
            Some(_) => unreachable!("shell quote state is bounded"),
            None => match character {
                '\'' | '"' => quote = Some(character),
                '\\' if characters.peek().is_some_and(|next| {
                    next.is_ascii_whitespace()
                        || matches!(next, '\'' | '"' | '\\' | ';' | '|' | '&' | '(' | ')')
                }) =>
                {
                    current.push(characters.next().expect("peeked escaped character"));
                }
                '\n' | '\r' => {
                    finish_shell_word(&mut current, &mut words);
                    finish_shell_command(&mut words, &mut commands);
                }
                character if character.is_ascii_whitespace() => {
                    finish_shell_word(&mut current, &mut words);
                }
                ';' | '|' | '&' | '(' | ')' => {
                    finish_shell_word(&mut current, &mut words);
                    finish_shell_command(&mut words, &mut commands);
                }
                _ => current.push(character),
            },
        }
    }
    finish_shell_word(&mut current, &mut words);
    finish_shell_command(&mut words, &mut commands);
    commands
}

fn executable_name(executable: &str) -> &str {
    executable.rsplit(['/', '\\']).next().unwrap_or(executable)
}

fn is_shell_assignment(word: &str) -> bool {
    shell_assignment_name(word).is_some()
}

fn shell_assignment_name(word: &str) -> Option<&str> {
    word.split_once('=').and_then(|(name, _)| {
        (!name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
        .then_some(name)
    })
}

fn invocation_from_words(words: &[String]) -> Option<CommandInvocation> {
    let mut index = 0;
    let mut environment_assignments = Vec::new();
    if words
        .first()
        .is_some_and(|word| executable_name(word).eq_ignore_ascii_case("env"))
    {
        index += 1;
        while let Some(word) = words.get(index) {
            if is_shell_assignment(word) {
                environment_assignments.push(word.clone());
                index += 1;
            } else if word.starts_with('-') {
                index += 1;
            } else {
                break;
            }
        }
    } else {
        while let Some(word) = words.get(index).filter(|word| is_shell_assignment(word)) {
            environment_assignments.push(word.clone());
            index += 1;
        }
    }
    if words
        .get(index)
        .is_some_and(|word| executable_name(word).eq_ignore_ascii_case("command"))
    {
        index += 1;
    }
    let executable = words.get(index)?.clone();
    Some(CommandInvocation {
        executable,
        arguments: words[index + 1..].to_vec(),
        environment_assignments,
    })
}

fn shell_payload(invocation: &CommandInvocation) -> Option<&str> {
    let shell = executable_name(&invocation.executable);
    let shell_supported = [
        "sh",
        "bash",
        "zsh",
        "powershell",
        "powershell.exe",
        "pwsh",
        "pwsh.exe",
        "cmd",
        "cmd.exe",
    ]
    .iter()
    .any(|candidate| shell.eq_ignore_ascii_case(candidate));
    if !shell_supported {
        return None;
    }
    invocation.arguments.windows(2).find_map(|pair| {
        ["-c", "-lc", "-command", "/c"]
            .iter()
            .any(|flag| pair[0].eq_ignore_ascii_case(flag))
            .then_some(pair[1].as_str())
    })
}

fn command_invocations(command: &str) -> Vec<CommandInvocation> {
    let mut invocations = Vec::new();
    let mut pending = vec![(command.to_string(), 0_usize)];
    while let Some((source, depth)) = pending.pop() {
        let mut bindings: Vec<(String, String)> = Vec::new();
        for words in parse_shell_commands(&source) {
            let words = words
                .iter()
                .map(|word| expand_shell_bindings(word, &bindings))
                .collect::<Vec<_>>();
            if !words.is_empty() && words.iter().all(|word| is_shell_assignment(word)) {
                for word in &words {
                    if let Some((name, value)) = word.split_once('=') {
                        bindings.retain(|(existing, _)| existing != name);
                        bindings.push((name.to_string(), value.to_string()));
                    }
                }
                continue;
            }
            let Some(invocation) = invocation_from_words(&words) else {
                continue;
            };
            if depth < 4 {
                if let Some(payload) = shell_payload(&invocation) {
                    pending.push((payload.to_string(), depth + 1));
                }
            }
            invocations.push(invocation);
        }
    }
    invocations
}

/// Expand `$NAME` and `${NAME}` occurrences from bindings assigned earlier in
/// the same command string. Agents legally abbreviate long declared paths
/// through such assignments; path-anchored lifecycle checks must see the
/// resolved text. Unknown variables stay untouched.
fn expand_shell_bindings(word: &str, bindings: &[(String, String)]) -> String {
    if bindings.is_empty() || !word.contains('$') {
        return word.to_string();
    }
    let mut expanded = word.to_string();
    let mut ordered = bindings.to_vec();
    ordered.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    for (name, value) in &ordered {
        expanded = expanded.replace(&format!("${{{name}}}"), value);
        expanded = expanded.replace(&format!("${name}"), value);
    }
    expanded
}

fn invocation_executable_is(invocation: &CommandInvocation, names: &[&str]) -> bool {
    let executable = executable_name(&invocation.executable);
    names
        .iter()
        .any(|name| executable.eq_ignore_ascii_case(name))
}

fn ait_command_invocations(command: &str) -> Vec<CommandInvocation> {
    command_invocations(command)
        .into_iter()
        .filter(|invocation| invocation_executable_is(invocation, &["ait", "ait.exe"]))
        .collect()
}

fn git_command_invocations(command: &str) -> Vec<CommandInvocation> {
    command_invocations(command)
        .into_iter()
        .filter(|invocation| invocation_executable_is(invocation, &["git", "git.exe"]))
        .collect()
}

fn git_invocation_overrides_metadata_context(invocation: &CommandInvocation) -> bool {
    if invocation.environment_assignments.iter().any(|assignment| {
        shell_assignment_name(assignment)
            .is_some_and(|name| matches!(name, "GIT_DIR" | "GIT_WORK_TREE"))
    }) {
        return true;
    }

    let mut index = 0;
    while let Some(argument) = invocation.arguments.get(index) {
        match argument.as_str() {
            "--git-dir" | "--work-tree" => return true,
            value if value.starts_with("--git-dir=") || value.starts_with("--work-tree=") => {
                return true;
            }
            "-C" | "-c" | "--exec-path" | "--namespace" => {
                index = index.saturating_add(2);
            }
            "--bare"
            | "--no-pager"
            | "--paginate"
            | "--no-replace-objects"
            | "--literal-pathspecs"
            | "--glob-pathspecs"
            | "--noglob-pathspecs"
            | "--icase-pathspecs" => {
                index = index.saturating_add(1);
            }
            value
                if value.starts_with("-C")
                    || value.starts_with("-c=")
                    || value.starts_with("--exec-path=")
                    || value.starts_with("--namespace=") =>
            {
                index = index.saturating_add(1);
            }
            _ => break,
        }
    }
    false
}

fn command_exports_git_metadata_context(command: &str) -> bool {
    command_invocations(command).iter().any(|invocation| {
        invocation_executable_is(invocation, &["export"])
            && invocation.arguments.iter().any(|argument| {
                shell_assignment_name(argument)
                    .is_some_and(|name| matches!(name, "GIT_DIR" | "GIT_WORK_TREE"))
            })
    })
}

fn git_subcommand_arguments(invocation: &CommandInvocation) -> &[String] {
    let mut index = 0;
    while let Some(argument) = invocation.arguments.get(index) {
        match argument.as_str() {
            "-C" | "-c" | "--exec-path" | "--git-dir" | "--work-tree" | "--namespace" => {
                index = index.saturating_add(2);
            }
            "--bare"
            | "--no-pager"
            | "--paginate"
            | "--no-replace-objects"
            | "--literal-pathspecs"
            | "--glob-pathspecs"
            | "--noglob-pathspecs"
            | "--icase-pathspecs" => {
                index = index.saturating_add(1);
            }
            value
                if value.starts_with("-C")
                    || value.starts_with("-c=")
                    || value.starts_with("--exec-path=")
                    || value.starts_with("--git-dir=")
                    || value.starts_with("--work-tree=")
                    || value.starts_with("--namespace=") =>
            {
                index = index.saturating_add(1);
            }
            _ => break,
        }
    }
    &invocation.arguments[index.min(invocation.arguments.len())..]
}

fn git_invocation_has_subcommand(invocation: &CommandInvocation, expected: &[&str]) -> bool {
    let arguments = git_subcommand_arguments(invocation);
    arguments
        .iter()
        .map(String::as_str)
        .zip(expected.iter().copied())
        .all(|(actual, expected)| actual == expected)
        && arguments.len() >= expected.len()
}

/// Git inspection surface available to a session that starts inside a
/// host-managed detached worktree. Anything that can move refs, the index, or
/// worktree registration remains host-owned and fails the measured lane.
fn git_invocation_is_app_managed_read_only(invocation: &CommandInvocation) -> bool {
    let arguments = git_subcommand_arguments(invocation);
    match arguments.first().map(String::as_str) {
        Some("status" | "diff" | "log" | "show" | "rev-parse" | "ls-files" | "grep") => true,
        Some("branch") => arguments
            .iter()
            .skip(1)
            .any(|argument| matches!(argument.as_str(), "--show-current" | "--list" | "-l")),
        Some("worktree") => arguments.get(1).map(String::as_str) == Some("list"),
        _ => false,
    }
}

fn invocation_has_subcommand(invocation: &CommandInvocation, expected: &[&str]) -> bool {
    invocation
        .arguments
        .iter()
        .map(String::as_str)
        .zip(expected.iter().copied())
        .all(|(actual, expected)| actual == expected)
        && invocation.arguments.len() >= expected.len()
}

fn invocation_has_option(invocation: &CommandInvocation, option: &str) -> bool {
    invocation.arguments.iter().any(|token| {
        token == option
            || token
                .strip_prefix(option)
                .is_some_and(|suffix| suffix.starts_with('='))
    })
}

fn command_invokes_ait(command: &str) -> bool {
    !ait_command_invocations(command).is_empty()
}

fn command_invokes_ait_server(command: &str) -> bool {
    command_invocations(command)
        .iter()
        .any(|invocation| invocation_executable_is(invocation, &["ait-server", "ait-server.exe"]))
}

fn command_invokes_git_vcs(command: &str) -> bool {
    !git_command_invocations(command).is_empty()
}

fn command_family(command: &str) -> &'static str {
    if command_invokes_ait(command) {
        "ait"
    } else if command.contains("git ") || command.trim_start().starts_with("git ") {
        "git"
    } else if command.contains("npm test")
        || command.contains("cargo test")
        || command.contains("self-test")
        || command.contains("release-check")
    {
        "validation"
    } else if command.contains(" rg ") || command.contains("rg --") || command.contains("grep ") {
        "search"
    } else if command.contains(" sed ")
        || command.contains(" cat ")
        || command.contains(" head ")
        || command.contains(" tail ")
    {
        "read"
    } else if command.contains("apply_patch") {
        "edit"
    } else {
        "other"
    }
}

fn summarize_optional_u64(
    samples: &[u64],
    bootstrap_resamples: usize,
    seed: u64,
) -> Result<Option<DistributionSummary>, String> {
    if samples.is_empty() {
        return Ok(None);
    }
    summarize_samples(
        &samples
            .iter()
            .map(|value| *value as f64)
            .collect::<Vec<_>>(),
        bootstrap_resamples,
        seed,
    )
    .map(Some)
}

fn paired_attempt_count(
    git_runs: &[&AgentTokenRunSummary],
    ait_runs: &[&AgentTokenRunSummary],
) -> usize {
    let git_attempts = git_runs
        .iter()
        .map(|run| run.attempt)
        .collect::<BTreeSet<_>>();
    let ait_attempts = ait_runs
        .iter()
        .map(|run| run.attempt)
        .collect::<BTreeSet<_>>();
    git_attempts.intersection(&ait_attempts).count()
}

fn paired_runs<'a>(
    git_runs: &[&'a AgentTokenRunSummary],
    ait_runs: &[&'a AgentTokenRunSummary],
) -> (Vec<&'a AgentTokenRunSummary>, Vec<&'a AgentTokenRunSummary>) {
    let git_by_attempt = git_runs
        .iter()
        .map(|run| (run.attempt, *run))
        .collect::<BTreeMap<_, _>>();
    let ait_by_attempt = ait_runs
        .iter()
        .map(|run| (run.attempt, *run))
        .collect::<BTreeMap<_, _>>();
    let mut paired_git = Vec::new();
    let mut paired_ait = Vec::new();
    for (attempt, git) in git_by_attempt {
        let Some(ait) = ait_by_attempt.get(&attempt).copied() else {
            continue;
        };
        paired_git.push(git);
        paired_ait.push(ait);
    }
    (paired_git, paired_ait)
}

fn failure_adjusted_effective_tokens(runs: &[&AgentTokenRunSummary]) -> Option<f64> {
    failure_adjusted_effective_measure(runs, |run| Some(run.usage.as_ref()?.provider_total_tokens))
}

fn failure_adjusted_effective_measure(
    runs: &[&AgentTokenRunSummary],
    measure: impl Fn(&AgentTokenRunSummary) -> Option<u64>,
) -> Option<f64> {
    let accepted = runs.iter().filter(|run| run.accepted_equivalent).count();
    if accepted == 0 {
        return None;
    }
    let total = runs
        .iter()
        .try_fold(0_u64, |total, run| total.checked_add(measure(run)?))?;
    Some(total as f64 / accepted as f64)
}

fn relative_reduction(baseline: Option<f64>, candidate: Option<f64>) -> Option<f64> {
    match (baseline, candidate) {
        (Some(baseline), Some(candidate)) if baseline > 0.0 => {
            Some(100.0 * (1.0 - candidate / baseline))
        }
        _ => None,
    }
}

fn complete_comparison_median(
    comparisons: &[AgentTokenModeComparison],
    required_count: usize,
    metric: impl Fn(&AgentTokenModeComparison) -> Option<f64>,
) -> Option<f64> {
    let values = comparisons.iter().filter_map(metric).collect::<Vec<_>>();
    (values.len() == required_count).then(|| quantile_r7_local(&values, 0.5))
}

fn acceptance_rate(runs: &[&AgentTokenRunSummary]) -> f64 {
    if runs.is_empty() {
        return 0.0;
    }
    runs.iter().filter(|run| run.accepted_equivalent).count() as f64 / runs.len() as f64
}

fn acceptance_rate_percentage_points(runs: &[&AgentTokenRunSummary]) -> f64 {
    if runs.is_empty() {
        return 0.0;
    }
    let accepted = runs.iter().filter(|run| run.accepted_equivalent).count();
    accepted as f64 * 100.0 / runs.len() as f64
}

fn acceptance_rate_deficit_exceeds_five_percentage_points(
    git_runs: &[&AgentTokenRunSummary],
    ait_runs: &[&AgentTokenRunSummary],
) -> bool {
    if git_runs.is_empty() || ait_runs.is_empty() {
        return false;
    }
    let git_accepted = git_runs
        .iter()
        .filter(|run| run.accepted_equivalent)
        .count() as i128;
    let ait_accepted = ait_runs
        .iter()
        .filter(|run| run.accepted_equivalent)
        .count() as i128;
    let git_count = git_runs.len() as i128;
    let ait_count = ait_runs.len() as i128;
    let deficit_numerator = git_accepted * ait_count - ait_accepted * git_count;
    100 * deficit_numerator > 5 * git_count * ait_count
}

fn bootstrap_failure_adjusted_savings(
    git_runs: &[&AgentTokenRunSummary],
    ait_runs: &[&AgentTokenRunSummary],
    resamples: usize,
    seed: u64,
) -> Option<Vec<f64>> {
    let git_by_attempt = git_runs
        .iter()
        .map(|run| (run.attempt, *run))
        .collect::<BTreeMap<_, _>>();
    let ait_by_attempt = ait_runs
        .iter()
        .map(|run| (run.attempt, *run))
        .collect::<BTreeMap<_, _>>();
    let pairs = git_by_attempt
        .iter()
        .filter_map(|(attempt, git)| ait_by_attempt.get(attempt).map(|ait| (*git, *ait)))
        .collect::<Vec<_>>();
    if pairs.is_empty() || resamples == 0 {
        return None;
    }
    let mut generator = DeterministicRng::new(seed);
    let mut samples = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut git_tokens = 0_u64;
        let mut ait_tokens = 0_u64;
        let mut git_accepted = 0_u64;
        let mut ait_accepted = 0_u64;
        for _ in 0..pairs.len() {
            let (git, ait) = pairs[generator.index(pairs.len())];
            git_tokens = git_tokens.saturating_add(git.usage.as_ref()?.provider_total_tokens);
            ait_tokens = ait_tokens.saturating_add(ait.usage.as_ref()?.provider_total_tokens);
            git_accepted += u64::from(git.accepted_equivalent);
            ait_accepted += u64::from(ait.accepted_equivalent);
        }
        if git_accepted == 0 || ait_accepted == 0 || git_tokens == 0 {
            continue;
        }
        let git_effective = git_tokens as f64 / git_accepted as f64;
        let ait_effective = ait_tokens as f64 / ait_accepted as f64;
        samples.push(100.0 * (1.0 - ait_effective / git_effective));
    }
    let minimum_usable = resamples.saturating_mul(9).div_ceil(10);
    (samples.len() >= minimum_usable).then_some(samples)
}

fn aggregate_bootstrap_medians(samples: &[Option<Vec<f64>>]) -> Option<Vec<f64>> {
    let complete = samples
        .iter()
        .map(Option::as_ref)
        .collect::<Option<Vec<_>>>()?;
    let count = complete.iter().map(|values| values.len()).min()?;
    if count == 0 {
        return None;
    }
    Some(
        (0..count)
            .map(|index| {
                quantile_r7_local(
                    &complete
                        .iter()
                        .map(|values| values[index])
                        .collect::<Vec<_>>(),
                    0.5,
                )
            })
            .collect(),
    )
}

fn quantile_r7_local(samples: &[f64], probability: f64) -> f64 {
    debug_assert!(!samples.is_empty());
    let mut ordered = samples.to_vec();
    ordered.sort_by(f64::total_cmp);
    if ordered.len() == 1 {
        return ordered[0];
    }
    let index = probability * (ordered.len() - 1) as f64;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;
    let fraction = index - lower as f64;
    ordered[lower] + (ordered[upper] - ordered[lower]) * fraction
}

fn stable_text_seed(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path, kind: &str) -> Result<T, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Failed to read {kind} {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Failed to decode {kind} {}: {error}", path.display()))
}

fn write_bytes_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "Failed to create {} without overwriting: {error}",
                path.display()
            )
        })?;
    output
        .write_all(bytes)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

fn require_text(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} must not be empty"))
    } else {
        Ok(())
    }
}

fn display_optional(value: Option<f64>) -> String {
    value
        .map(|number| format!("{number:.1}"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn display_optional_percent(value: Option<f64>) -> String {
    value
        .map(|number| format!("{number:.2}%"))
        .unwrap_or_else(|| "n/a".to_string())
}

#[cfg(test)]
mod tests {
    fn git_lifecycle_commands(worktree_start: &str) -> Vec<String> {
        let worktree = "/evidence/run/git-worktree-runtime/git-task-worktree";
        vec![
            "/usr/bin/git status --short --branch".to_string(),
            format!(
                "/bin/zsh -lc '/usr/bin/git worktree add -b benchmark-task {worktree}{worktree_start} && /usr/bin/git status --short --branch'"
            ),
            "npm test".to_string(),
            "/usr/bin/git commit -m candidate".to_string(),
            "/usr/bin/git merge --ff-only benchmark-task".to_string(),
            format!("/usr/bin/git worktree remove {worktree}"),
            "/usr/bin/git branch -d benchmark-task".to_string(),
        ]
    }

    #[test]
    fn implicit_git_worktree_main_requires_steady_state_start_proof() {
        let explicit = validate_agent_token_command_list_with_git_start_proof(
            git_lifecycle_commands(" main"),
            "explicit-main",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            false,
        )
        .unwrap();
        assert!(explicit.valid, "{:?}", explicit.errors);

        let unproven = validate_agent_token_command_list_with_git_start_proof(
            git_lifecycle_commands(""),
            "implicit-unproven",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            false,
        )
        .unwrap();
        assert!(!unproven.valid);
        assert!(unproven
            .errors
            .iter()
            .any(|error| error.contains("without a runner-proven clean main HEAD")));

        let proven = validate_agent_token_command_list_with_git_start_proof(
            git_lifecycle_commands(""),
            "implicit-proven",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            true,
        )
        .unwrap();
        assert!(proven.valid, "{:?}", proven.errors);
        assert!(proven
            .observed_required_commands
            .iter()
            .any(|command| command.contains("runner-proven clean main HEAD")));

        let first_use = validate_agent_token_command_list_with_git_start_proof(
            git_lifecycle_commands(""),
            "implicit-first-use",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::FirstUseTotalCost,
            true,
        )
        .unwrap();
        assert!(first_use
            .errors
            .iter()
            .any(|error| error.contains("without a runner-proven clean main HEAD")));

        let alternate_start = validate_agent_token_command_list_with_git_start_proof(
            git_lifecycle_commands(" feature"),
            "alternate-start",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            true,
        )
        .unwrap();
        assert!(alternate_start.errors.iter().any(|error| error
            .contains("did not create the declared benchmark-task linked worktree from main")));
    }

    #[test]
    fn claude_transcript_extracts_bash_commands_and_validates() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude-stream-sample.jsonl");
        // The sample transcript is a plain echo probe, so validating it as a
        // measured lane must fail closed (no task start/finish), while the
        // command extraction itself must find the exact Bash invocation.
        let transcript = extract_and_validate_claude_transcript(
            &fixture,
            "claude-sample",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .expect("claude transcript parses");
        assert_eq!(transcript.commands, vec!["echo hello".to_string()]);
        assert!(!transcript.valid);
        assert!(transcript
            .errors
            .iter()
            .any(|error| error.contains("required command")));
    }

    #[test]
    fn variable_bound_paths_resolve_for_declared_lifecycle_matching() {
        // The captured GD-02 closing command: the executed worktree removal
        // was previously misjudged as missing because the declared path was
        // carried through same-command shell variables.
        let closing = concat!(
            "M=/evidence/run-gd-02-git/workspace\n",
            "W=/evidence/run-gd-02-git/git-worktree-runtime/git-task-worktree\n",
            "/usr/bin/git -C \"$M\" merge --ff-only benchmark-task\n",
            "/usr/bin/git -C \"$M\" worktree remove \"$W\"\n",
            "/usr/bin/git -C \"$M\" branch -d benchmark-task"
        );
        let commands = vec![
            "/usr/bin/git worktree add -b benchmark-task /evidence/run-gd-02-git/git-worktree-runtime/git-task-worktree main".to_string(),
            "npm test".to_string(),
            "/usr/bin/git commit -m candidate".to_string(),
            closing.to_string(),
        ];
        let transcript = validate_agent_token_command_list(
            commands,
            "variable-paths",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .expect("transcript validates");
        assert!(
            !transcript
                .errors
                .iter()
                .any(|error| error.contains("did not remove the declared")),
            "variable-bound removal must be recognized: {:?}",
            transcript.errors
        );

        // Unknown variables must not silently satisfy a declared-path check.
        let transcript = validate_agent_token_command_list(
            vec![
                "/usr/bin/git worktree add -b benchmark-task /evidence/x/git-worktree-runtime/git-task-worktree main".to_string(),
                "npm test".to_string(),
                "/usr/bin/git commit -m candidate".to_string(),
                "/usr/bin/git worktree remove \"$UNSET\"".to_string(),
            ],
            "unset-variable",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .expect("transcript validates");
        assert!(transcript
            .errors
            .iter()
            .any(|error| error.contains("did not remove the declared")));
    }

    #[test]
    fn ait_readonly_inspection_is_informational_but_remote_authority_stays_forbidden() {
        // Symmetric with the Git treatment's unrestricted status/diff/log:
        // read-only local inspection must not consume lifecycle budget.
        let transcript = validate_agent_token_command_list(
            vec![
                "/opt/ait task start --title t --intent i --local --json".to_string(),
                "/opt/ait status --json".to_string(),
                "/opt/ait diff --stat".to_string(),
                "/opt/ait blame src/game.js".to_string(),
                "npm test".to_string(),
                "/opt/ait task finish LT-0001 --message m --local".to_string(),
            ],
            "readonly-inspection",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .expect("transcript validates");
        assert!(
            transcript.valid,
            "read-only inspection must be admitted: {:?}",
            transcript.errors
        );

        // Remote authority is never informational.
        let transcript = validate_agent_token_command_list(
            vec![
                "/opt/ait task start --title t --intent i --local --json".to_string(),
                "/opt/ait status --remote=origin".to_string(),
                "npm test".to_string(),
                "/opt/ait task finish LT-0001 --message m --local".to_string(),
            ],
            "remote-inspection",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .expect("transcript validates");
        assert!(!transcript.valid);
        assert!(transcript
            .errors
            .iter()
            .any(|error| error.contains("--remote")));

        // Management surfaces stay forbidden even though they are read-only.
        let transcript = validate_agent_token_command_list(
            vec![
                "/opt/ait task start --title t --intent i --local --json".to_string(),
                "/opt/ait task list --all".to_string(),
                "npm test".to_string(),
                "/opt/ait task finish LT-0001 --message m --local".to_string(),
            ],
            "management-surface",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .expect("transcript validates");
        assert!(!transcript.valid);
    }

    #[test]
    fn same_tool_help_introspection_is_admitted_and_cross_tool_stays_forbidden() {
        // The exact r5-lane phrasing must now validate cleanly alongside the
        // frozen lifecycle.
        let commands = vec![
            "/Users/weita/.local/bin/ait --help 2>&1 | head -50".to_string(),
            "/Users/weita/.local/bin/ait task start --title t --intent i --local --json"
                .to_string(),
            "npm test 2>&1 | tail -25".to_string(),
            "/Users/weita/.local/bin/ait task finish LT-0001 --message m --local --json"
                .to_string(),
        ];
        let transcript = validate_agent_token_command_list(
            commands,
            "help-admitted",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .expect("transcript validates");
        assert!(
            transcript.valid,
            "help introspection must be admitted: {:?}",
            transcript.errors
        );

        // Subcommand help forms are informational too.
        let transcript = validate_agent_token_command_list(
            vec![
                "/opt/ait workflow ready --help".to_string(),
                "/opt/ait help task".to_string(),
                "ait task start --title t --intent i --local --json".to_string(),
                "npm test".to_string(),
                "ait task finish LT-0001 --message m --local".to_string(),
            ],
            "help-forms",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .expect("transcript validates");
        assert!(transcript.valid, "{:?}", transcript.errors);

        // Cross-tool help stays prohibited: git help inside AIT mode.
        let transcript = validate_agent_token_command_list(
            vec![
                "/usr/bin/git --help".to_string(),
                "ait task start --title t --intent i --local --json".to_string(),
                "npm test".to_string(),
                "ait task finish LT-0001 --message m --local".to_string(),
            ],
            "cross-tool-help",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .expect("transcript validates");
        assert!(!transcript.valid);

        // Git mode admits its own help the same way.
        let transcript = validate_agent_token_command_list(
            vec![
                "/usr/bin/git worktree --help".to_string(),
                "/usr/bin/git worktree add -b benchmark-task /tmp/wt main".to_string(),
                "/usr/bin/git commit -m candidate".to_string(),
                "/usr/bin/git merge --ff-only benchmark-task".to_string(),
                "/usr/bin/git worktree remove /tmp/wt".to_string(),
                "/usr/bin/git branch -d benchmark-task".to_string(),
            ],
            "git-help",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .expect("transcript validates");
        assert!(
            !transcript
                .errors
                .iter()
                .any(|error| error.contains("--help")),
            "git help must not be flagged: {:?}",
            transcript.errors
        );
    }

    #[test]
    fn claude_transcript_fails_closed_on_out_of_surface_tool_use() {
        let temp = tempfile::tempdir().unwrap();
        let stream = temp.path().join("claude-events.raw.jsonl");
        std::fs::write(
            &stream,
            concat!(
                "{\"type\":\"assistant\",\"message\":{\"content\":[",
                "{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Bash\",",
                "\"input\":{\"command\":\"echo hi\"}}]}}\n",
                "{\"type\":\"assistant\",\"message\":{\"content\":[",
                "{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"Task\",",
                "\"input\":{\"prompt\":\"spawn\"}}]}}\n",
                "{\"type\":\"result\",\"subtype\":\"success\",\"num_turns\":2}\n"
            ),
        )
        .unwrap();
        let transcript = extract_and_validate_claude_transcript(
            &stream,
            "claude-surface",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .expect("claude transcript parses");
        assert!(!transcript.valid);
        assert!(transcript.errors.iter().any(|error| {
            error.contains("outside the declared measured surface") && error.contains("Task")
        }));
    }

    #[test]
    fn claude_secondary_metrics_count_edits_rejections_and_tool_output() {
        let temp = tempfile::tempdir().unwrap();
        let stream = temp.path().join("claude-events.raw.jsonl");
        std::fs::write(
            &stream,
            concat!(
                "{\"type\":\"system\",\"subtype\":\"init\"}\n",
                "{\"type\":\"assistant\",\"message\":{\"content\":[",
                "{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Bash\",",
                "\"input\":{\"command\":\"git status\"}}]}}\n",
                "{\"type\":\"user\",\"message\":{\"content\":[",
                "{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_1\",",
                "\"content\":\"0123456789\"}]}}\n",
                "{\"type\":\"assistant\",\"message\":{\"content\":[",
                "{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"Edit\",",
                "\"input\":{\"file_path\":\"a.txt\"}}]}}\n",
                "{\"type\":\"user\",\"message\":{\"content\":[",
                "{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_2\",",
                "\"is_error\":true,\"content\":\"edit failed\"}]}}\n",
                "{\"type\":\"assistant\",\"message\":{\"content\":[",
                "{\"type\":\"tool_use\",\"id\":\"toolu_3\",\"name\":\"Write\",",
                "\"input\":{\"file_path\":\"a.txt\"}}]}}\n",
                "{\"type\":\"user\",\"message\":{\"content\":[",
                "{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_3\",",
                "\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}]}}\n",
                "{\"type\":\"result\",\"subtype\":\"success\",\"num_turns\":4}\n"
            ),
        )
        .unwrap();
        let transcript = AgentTokenCommandTranscript {
            contract: AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
            run_id: "claude-metrics".to_string(),
            mode: AgentTokenMode::AitLinearSingleSession,
            accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
            command_count: 1,
            commands: vec!["git status".to_string()],
            valid: false,
            errors: Vec::new(),
            observed_required_commands: Vec::new(),
        };
        let metrics = extract_agent_token_claude_secondary_metrics(&stream, &transcript)
            .expect("claude metrics extract");
        assert_eq!(metrics.model_calls, 3);
        assert_eq!(metrics.agent_turns, 4);
        assert_eq!(metrics.apply_patch_attempts, 2);
        assert_eq!(metrics.apply_patch_rejected_attempts, 1);
        assert_eq!(metrics.file_change_items, 1);
        // "0123456789" + "edit failed" + "ok" from the three tool results.
        assert_eq!(metrics.tool_output_bytes, 10 + 11 + 2);
        // Shared command statistics run over the validated transcript.
        assert_eq!(metrics.command_tool_calls, 1);
        assert_eq!(metrics.repository_query_calls, 1);
    }

    #[test]
    fn claude_usage_import_proves_model_purity_and_preserves_refusal() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("claude-pure.jsonl");
        let model = AgentTokenModelPin {
            provider: "anthropic".to_string(),
            model_id: "claude-fable-5".to_string(),
            model_revision: "sample".to_string(),
            reasoning_effort: "max".to_string(),
        };
        let stream = [
            serde_json::json!({
                "type": "system",
                "subtype": "init",
                "model": "claude-fable-5",
                "tools": ["Bash", "Read", "Grep", "Glob", "Edit", "Write"],
                "mcp_servers": [],
            }),
            serde_json::json!({
                "type": "assistant",
                "message": {"model": "claude-fable-5", "content": [{"type": "text", "text": "cannot comply"}]},
            }),
            serde_json::json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "terminal_reason": "completed",
                "stop_reason": "refusal",
                "num_turns": 1,
                "usage": {
                    "input_tokens": 18,
                    "cache_read_input_tokens": 54_125,
                    "cache_creation_input_tokens": 8_014,
                    "output_tokens": 159,
                    "output_tokens_details": {"thinking_tokens": 78},
                },
                "modelUsage": {
                    "claude-fable-5": {
                        "inputTokens": 18,
                        "cacheReadInputTokens": 54_125,
                        "cacheCreationInputTokens": 8_014,
                        "outputTokens": 159,
                        "canonicalModel": "claude-fable-5",
                    }
                },
            }),
        ]
        .into_iter()
        .map(|event| event.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        fs::write(&fixture, format!("{stream}\n")).unwrap();
        let imported = import_claude_usage_with_outcome(
            &fixture,
            "claude-sample",
            "GD-00",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            &model,
            AgentTokenClaudeModelAdmission::Strict,
        )
        .expect("claude usage imports");
        let usage = imported.usage;
        assert_eq!(usage.input_tokens, 18 + 54_125 + 8_014);
        assert_eq!(usage.cached_input_tokens, Some(54_125));
        assert_eq!(usage.cache_write_input_tokens, Some(8_014));
        assert_eq!(usage.output_tokens, 159);
        assert_eq!(usage.reasoning_tokens, Some(78));
        assert_eq!(
            usage.provider_total_tokens,
            usage.input_tokens + usage.output_tokens
        );
        assert_eq!(usage.completed_turns, 1);
        assert_eq!(
            usage.usage_provenance,
            "claude-code-stream-json:result+model-purity"
        );
        assert!(imported.provider_refusal);
        assert_eq!(imported.provider_stop_reason, "refusal");
    }

    #[test]
    fn claude_usage_import_rejects_any_additional_model() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("claude-mixed.jsonl");
        fs::write(
            &fixture,
            concat!(
                "{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-fable-5\",\"tools\":[\"Bash\",\"Read\",\"Grep\",\"Glob\",\"Edit\",\"Write\"],\"mcp_servers\":[]}\n",
                "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-fable-5\",\"content\":[]}}\n",
                "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"terminal_reason\":\"completed\",\"stop_reason\":\"end_turn\",\"num_turns\":1,\"usage\":{\"input_tokens\":1,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":1},\"modelUsage\":{\"claude-fable-5\":{\"inputTokens\":1,\"cacheReadInputTokens\":0,\"cacheCreationInputTokens\":0,\"outputTokens\":1,\"canonicalModel\":\"claude-fable-5\"},\"claude-haiku-4-5-20251001\":{\"inputTokens\":1,\"cacheReadInputTokens\":0,\"cacheCreationInputTokens\":0,\"outputTokens\":1,\"canonicalModel\":\"claude-haiku-4-5\"}}}\n"
            ),
        )
        .unwrap();
        let model = AgentTokenModelPin {
            provider: "anthropic".to_string(),
            model_id: "claude-fable-5".to_string(),
            model_revision: "sample".to_string(),
            reasoning_effort: "max".to_string(),
        };
        let error = import_claude_usage(
            &fixture,
            "claude-mixed",
            "GD-01",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            &model,
        )
        .unwrap_err();
        assert!(
            error.contains("must contain only the pinned model"),
            "{error}"
        );
    }

    #[test]
    fn as_shipped_admission_accepts_provider_fallback_and_counts_every_model() {
        // The runner supplies no --fallback-model, yet the provider can still
        // switch mid-session. Under as_shipped the campaign keeps running and
        // the terminal usage total, which already sums every served model, is
        // preserved so no provider-reported token is dropped.
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("claude-fallback.jsonl");
        fs::write(
            &fixture,
            concat!(
                "{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-fable-5\",\"tools\":[\"Bash\",\"Read\",\"Grep\",\"Glob\",\"Edit\",\"Write\"],\"mcp_servers\":[]}\n",
                "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-fable-5\",\"content\":[]}}\n",
                "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-opus-5\",\"content\":[{\"type\":\"fallback\",\"from\":{\"model\":\"claude-fable-5\"},\"to\":{\"model\":\"claude-opus-5\"}}]}}\n",
                "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"terminal_reason\":\"completed\",\"stop_reason\":\"end_turn\",\"num_turns\":1,\"usage\":{\"input_tokens\":30,\"cache_read_input_tokens\":700,\"cache_creation_input_tokens\":40,\"output_tokens\":9},\"modelUsage\":{\"claude-fable-5\":{\"inputTokens\":20,\"cacheReadInputTokens\":500,\"cacheCreationInputTokens\":30,\"outputTokens\":6,\"canonicalModel\":\"claude-fable-5\"},\"claude-opus-5\":{\"inputTokens\":10,\"cacheReadInputTokens\":200,\"cacheCreationInputTokens\":10,\"outputTokens\":3,\"canonicalModel\":\"claude-opus-5\"}}}\n"
            ),
        )
        .unwrap();
        let model = AgentTokenModelPin {
            provider: "anthropic".to_string(),
            model_id: "claude-fable-5".to_string(),
            model_revision: "sample".to_string(),
            reasoning_effort: "max".to_string(),
        };
        let args = (
            "claude-fallback",
            "GD-01",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        );

        let strict = import_claude_usage_with_outcome(
            &fixture,
            args.0,
            args.1,
            args.2,
            args.3,
            &model,
            AgentTokenClaudeModelAdmission::Strict,
        )
        .unwrap_err();
        assert!(strict.contains("differs from the pin"), "{strict}");

        let imported = import_claude_usage_with_outcome(
            &fixture,
            args.0,
            args.1,
            args.2,
            args.3,
            &model,
            AgentTokenClaudeModelAdmission::AsShipped,
        )
        .expect("as_shipped admits the fallback session");
        // 30 + 700 + 40 direct/cached/written input, plus 9 output, across both
        // served models.
        assert_eq!(imported.usage.input_tokens, 770);
        assert_eq!(imported.usage.output_tokens, 9);
        assert_eq!(imported.usage.provider_total_tokens, 779);
        assert_eq!(
            imported.usage.usage_provenance,
            "claude-code-stream-json:result+served-model-sum"
        );
        assert!(imported.fallback_observed);
        assert_eq!(imported.served_models.len(), 2);
        assert_eq!(imported.served_models[0].model_id, "claude-fable-5");
        assert_eq!(imported.served_models[0].provider_total_tokens, 556);
        assert_eq!(imported.served_models[1].model_id, "claude-opus-5");
        assert_eq!(imported.served_models[1].provider_total_tokens, 223);
    }

    #[test]
    fn as_shipped_admission_rejects_terminal_and_per_model_usage_disagreement() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("claude-fallback-drift.jsonl");
        fs::write(
            &fixture,
            concat!(
                "{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-fable-5\",\"tools\":[\"Bash\",\"Read\",\"Grep\",\"Glob\",\"Edit\",\"Write\"],\"mcp_servers\":[]}\n",
                "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-fable-5\",\"content\":[]}}\n",
                "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-opus-5\",\"content\":[{\"type\":\"fallback\"}]}}\n",
                "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"terminal_reason\":\"completed\",\"stop_reason\":\"end_turn\",\"num_turns\":1,\"usage\":{\"input_tokens\":31,\"cache_read_input_tokens\":700,\"cache_creation_input_tokens\":40,\"output_tokens\":9},\"modelUsage\":{\"claude-fable-5\":{\"inputTokens\":20,\"cacheReadInputTokens\":500,\"cacheCreationInputTokens\":30,\"outputTokens\":6,\"canonicalModel\":\"claude-fable-5\"},\"claude-opus-5\":{\"inputTokens\":10,\"cacheReadInputTokens\":200,\"cacheCreationInputTokens\":10,\"outputTokens\":3,\"canonicalModel\":\"claude-opus-5\"}}}\n"
            ),
        )
        .unwrap();
        let model = AgentTokenModelPin {
            provider: "anthropic".to_string(),
            model_id: "claude-fable-5".to_string(),
            model_revision: "sample".to_string(),
            reasoning_effort: "max".to_string(),
        };
        let error = import_claude_usage_with_outcome(
            &fixture,
            "claude-fallback-drift",
            "GD-01",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            &model,
            AgentTokenClaudeModelAdmission::AsShipped,
        )
        .unwrap_err();
        assert!(
            error.contains("summed modelUsage field inputTokens differs"),
            "{error}"
        );
    }

    #[test]
    fn claude_usage_import_rejects_prompt_suggestion_events() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("claude-prompt-suggestion.jsonl");
        fs::write(
            &fixture,
            concat!(
                "{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-fable-5\",\"tools\":[\"Bash\",\"Read\",\"Grep\",\"Glob\",\"Edit\",\"Write\"],\"mcp_servers\":[]}\n",
                "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-fable-5\",\"content\":[]}}\n",
                "{\"type\":\"prompt_suggestion\",\"suggestion\":\"continue\"}\n",
                "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"terminal_reason\":\"completed\",\"stop_reason\":\"end_turn\",\"num_turns\":1,\"usage\":{\"input_tokens\":1,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":1},\"modelUsage\":{\"claude-fable-5\":{\"inputTokens\":1,\"cacheReadInputTokens\":0,\"cacheCreationInputTokens\":0,\"outputTokens\":1,\"canonicalModel\":\"claude-fable-5\"}}}\n"
            ),
        )
        .unwrap();
        let model = AgentTokenModelPin {
            provider: "anthropic".to_string(),
            model_id: "claude-fable-5".to_string(),
            model_revision: "sample".to_string(),
            reasoning_effort: "max".to_string(),
        };
        let error = import_claude_usage(
            &fixture,
            "claude-prompt-suggestion",
            "GD-01",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            &model,
        )
        .unwrap_err();
        assert!(error.contains("prompt suggestion"), "{error}");
    }

    use super::*;

    #[test]
    fn missing_run_summary_is_skipped_only_for_the_exact_explicit_id() {
        let temp = tempfile::tempdir().unwrap();
        let runs = temp.path().join("runs");
        fs::create_dir(&runs).unwrap();
        fs::create_dir(runs.join("interrupted-run")).unwrap();

        assert!(load_agent_token_raw_run_summaries(temp.path()).is_err());
        assert!(load_agent_token_raw_run_summaries_with_allowed_missing(
            temp.path(),
            Some("different-run")
        )
        .is_err());
        assert!(load_agent_token_raw_run_summaries_with_allowed_missing(
            temp.path(),
            Some("interrupted-run")
        )
        .unwrap()
        .is_empty());
    }

    fn model() -> AgentTokenModelPin {
        AgentTokenModelPin {
            provider: "openai".to_string(),
            model_id: "gpt-test".to_string(),
            model_revision: "test-revision".to_string(),
            reasoning_effort: "medium".to_string(),
        }
    }

    fn write_test_transcript(path: &Path, commands: &[&str]) {
        let body = commands
            .iter()
            .enumerate()
            .map(|(index, command)| {
                serde_json::json!({
                    "type": "item.completed",
                    "item": {
                        "id": format!("item-{index}"),
                        "type": "command_execution",
                        "command": command,
                    }
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, format!("{body}\n")).unwrap();
    }

    fn validation_manifest(
        scope: AgentTokenCampaignScope,
        project_doc_max_bytes: usize,
        fixture_manifest: PathBuf,
    ) -> AgentTokenCampaignManifest {
        AgentTokenCampaignManifest {
            contract: AGENT_TOKEN_CAMPAIGN_CONTRACT.to_string(),
            campaign_id: "project-doc-diagnostic".to_string(),
            protocol_revision: AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
            campaign_scope: scope,
            accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
            ait_sprint_mode: AgentTokenAitSprintMode::Off,
            ait_edit_root_mode: crate::agent_token::AgentTokenAitEditRootMode::Explicit,
            git_worktree_mode: AgentTokenGitWorktreeMode::AgentManaged,
            claude_model_admission: crate::agent_token::AgentTokenClaudeModelAdmission::Strict,
            functional_replacement_policy: AgentTokenFunctionalReplacementPolicy::None,
            seed: 42,
            attempts_per_cell: scope.minimum_attempts(),
            workload_ids: ["GD-01", "GD-02", "GD-03", "GD-04", "GD-05"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            modes: vec![
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ],
            model: model(),
            runtime: AgentTokenRuntime {
                executor: AgentTokenExecutor::default(),
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
                fixture_manifest,
                run_timeout_seconds: 60,
                ait_first_use_worktree_add_dir: None,
                project_doc_max_bytes,
            },
            cache_class: "provider_default".to_string(),
            network_policy: "disabled_except_loopback".to_string(),
            tool_policy: "codex_shell_only".to_string(),
            bootstrap_resamples: 1_000,
            limitations: Vec::new(),
        }
    }

    #[test]
    fn project_document_loading_is_bounded_smoke_only_and_publication_blocked() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("fixture.json");
        fs::write(&fixture, "{}\n").unwrap();

        let smoke = validation_manifest(AgentTokenCampaignScope::Smoke, 8_192, fixture.clone());
        validate_agent_token_campaign(&smoke).unwrap();
        let schedule = build_agent_token_schedule(&smoke);
        let report = build_agent_token_report(&smoke, &schedule, &[]).unwrap();
        assert_eq!(report.project_doc_max_bytes, 8_192);
        assert!(!report.claim_eligible);
        assert!(report.blockers.iter().any(|blocker| blocker
            == "Project-document-loading diagnostic evidence is not publication eligible"));
        assert!(report
            .limitations
            .iter()
            .any(|limitation| limitation.contains("project_doc_max_bytes=8192")));

        let complete =
            validation_manifest(AgentTokenCampaignScope::Complete, 8_192, fixture.clone());
        assert!(validate_agent_token_campaign(&complete)
            .unwrap_err()
            .contains("only for smoke diagnostics"));
        let oversized = validation_manifest(AgentTokenCampaignScope::Smoke, 65_537, fixture);
        assert!(validate_agent_token_campaign(&oversized)
            .unwrap_err()
            .contains("must not exceed 65536"));
    }

    #[test]
    fn sprint_on_is_an_exact_five_workload_scope_with_smoke_diagnostic_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("fixture.json");
        fs::write(&fixture, "{}\n").unwrap();
        let mut manifest = validation_manifest(AgentTokenCampaignScope::Smoke, 0, fixture.clone());
        manifest.ait_sprint_mode = AgentTokenAitSprintMode::On;

        validate_agent_token_campaign(&manifest).unwrap();
        let schedule = build_agent_token_schedule(&manifest);
        assert_eq!(schedule.entry_count, 10);
        let report = build_agent_token_report(&manifest, &schedule, &[]).unwrap();
        assert_eq!(report.ait_sprint_mode, AgentTokenAitSprintMode::On);
        assert!(!report.claim_eligible);
        assert!(report
            .blockers
            .iter()
            .any(|blocker| blocker
                == "AIT sprint-on diagnostic evidence is not publication eligible"));

        let mut incomplete = manifest.clone();
        incomplete.workload_ids.pop();
        assert!(validate_agent_token_campaign(&incomplete)
            .unwrap_err()
            .contains("must contain all five workloads"));

        let mut complete = manifest.clone();
        complete.campaign_scope = AgentTokenCampaignScope::Complete;
        complete.attempts_per_cell = complete.campaign_scope.minimum_attempts();
        validate_agent_token_campaign(&complete).unwrap();
        let complete_schedule = build_agent_token_schedule(&complete);
        assert_eq!(
            complete_schedule.entry_count,
            AGENT_TOKEN_COMPLETE_SCHEDULED_RUNS
        );
        let complete_report = build_agent_token_report(&complete, &complete_schedule, &[]).unwrap();
        assert!(complete_report
            .blockers
            .iter()
            .all(|blocker| blocker
                != "AIT sprint-on diagnostic evidence is not publication eligible"));

        let mut replacement_complete = complete.clone();
        replacement_complete.functional_replacement_policy =
            AgentTokenFunctionalReplacementPolicy::FirstValidUnacceptedLaneOnce;
        validate_agent_token_campaign(&replacement_complete).unwrap();
        let replacement_report = build_agent_token_report(
            &replacement_complete,
            &build_agent_token_schedule(&replacement_complete),
            &[],
        )
        .unwrap();
        assert_eq!(
            replacement_report.functional_replacement_policy,
            AgentTokenFunctionalReplacementPolicy::FirstValidUnacceptedLaneOnce
        );

        let mut replacement_smoke = manifest.clone();
        replacement_smoke.functional_replacement_policy =
            AgentTokenFunctionalReplacementPolicy::FirstValidUnacceptedLaneOnce;
        assert!(validate_agent_token_campaign(&replacement_smoke)
            .unwrap_err()
            .contains("only for complete campaigns"));

        let mut first_use = manifest.clone();
        first_use.accounting_profile = AgentTokenAccountingProfile::FirstUseTotalCost;
        first_use.runtime.ait_first_use_worktree_add_dir = Some(temp.path().to_path_buf());
        assert!(validate_agent_token_campaign(&first_use)
            .unwrap_err()
            .contains("requires steady_state_task_cost"));

        let mut mixed = manifest;
        mixed.runtime.project_doc_max_bytes = 8_192;
        assert!(validate_agent_token_campaign(&mixed)
            .unwrap_err()
            .contains("cannot mix project-document-loading treatment"));
    }

    #[test]
    fn app_equivalent_managed_worktree_is_a_pinned_complete_200_axis() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("fixture.json");
        fs::write(&fixture, "{}\n").unwrap();
        let mut managed =
            validation_manifest(AgentTokenCampaignScope::Complete, 0, fixture.clone());
        managed.ait_edit_root_mode = AgentTokenAitEditRootMode::Returned;
        managed.git_worktree_mode = AgentTokenGitWorktreeMode::CodexAppEquivalentManaged;

        validate_agent_token_campaign(&managed).unwrap();
        let schedule = build_agent_token_schedule(&managed);
        assert_eq!(schedule.entry_count, 200);
        assert_eq!(schedule.entries.len(), 200);
        assert_eq!(schedule.entries[..10].len(), 10);
        for pair in schedule.entries[..10].chunks_exact(2) {
            assert_eq!(pair[0].workload_id, pair[1].workload_id);
            assert_eq!(pair[0].attempt, pair[1].attempt);
            assert_eq!(
                pair.iter().map(|entry| entry.mode).collect::<BTreeSet<_>>(),
                BTreeSet::from([
                    AgentTokenMode::GitLinearSingleSession,
                    AgentTokenMode::AitLinearSingleSession,
                ])
            );
        }
        assert_eq!(
            serde_json::to_vec(&schedule.entries[..10]).unwrap(),
            serde_json::to_vec(&build_agent_token_schedule(&managed).entries[..10]).unwrap()
        );

        let managed_report = build_agent_token_report(&managed, &schedule, &[]).unwrap();
        assert_eq!(
            managed_report.git_worktree_mode,
            AgentTokenGitWorktreeMode::CodexAppEquivalentManaged
        );
        assert!(managed_report
            .limitations
            .iter()
            .any(|limitation| limitation.contains("private worktree IPC was not invoked")));

        let mut historical = managed.clone();
        historical.git_worktree_mode = AgentTokenGitWorktreeMode::AgentManaged;
        let historical_report =
            build_agent_token_report(&historical, &build_agent_token_schedule(&historical), &[])
                .unwrap();
        assert!(
            compare_agent_token_reports(&historical_report, &managed_report)
                .blockers
                .contains(&"Git worktree ownership differs".to_string())
        );

        let mut predecessor = managed.clone();
        predecessor.protocol_revision =
            AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION.to_string();
        assert!(validate_agent_token_campaign(&predecessor)
            .unwrap_err()
            .contains(AGENT_TOKEN_PROTOCOL_REVISION));

        let mut preflight_predecessor = managed.clone();
        preflight_predecessor.protocol_revision =
            AGENT_TOKEN_MANAGED_WORKTREE_PREFLIGHT_PREDECESSOR_PROTOCOL_REVISION.to_string();
        validate_agent_token_campaign_source(&preflight_predecessor)
            .expect(".47 zero-lane managed-worktree evidence remains readable");
        assert!(validate_agent_token_campaign(&preflight_predecessor)
            .unwrap_err()
            .contains(AGENT_TOKEN_PROTOCOL_REVISION));

        let mut explicit_edit_root = managed.clone();
        explicit_edit_root.ait_edit_root_mode = AgentTokenAitEditRootMode::Explicit;
        assert!(validate_agent_token_campaign(&explicit_edit_root)
            .unwrap_err()
            .contains("without --edit-root"));

        let mut claude = managed;
        claude.runtime.executor = AgentTokenExecutor::Claude;
        claude.runtime.claude_program = Some(PathBuf::from("claude"));
        claude.runtime.executor_version = Some("test".to_string());
        claude.model.provider = "anthropic".to_string();
        claude.tool_policy = "claude_code_local_tools".to_string();
        assert!(validate_agent_token_campaign(&claude)
            .unwrap_err()
            .contains("requires the Codex executor"));
    }

    #[test]
    fn active_claude_campaign_requires_an_exact_executor_version_pin() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("fixture.json");
        fs::write(&fixture, "{}\n").unwrap();
        let mut manifest = validation_manifest(AgentTokenCampaignScope::Smoke, 0, fixture);
        manifest.runtime.executor = AgentTokenExecutor::Claude;
        manifest.runtime.claude_program = Some(PathBuf::from("claude"));
        manifest.model.provider = "anthropic".to_string();
        manifest.tool_policy = "claude_code_local_tools".to_string();

        assert!(validate_agent_token_campaign(&manifest)
            .unwrap_err()
            .contains("runtime.executor_version"));
        manifest.runtime.executor_version = Some("2.1.235 (Claude Code)".to_string());
        validate_agent_token_campaign(&manifest).unwrap();

        let mut predecessor = manifest.clone();
        predecessor.protocol_revision =
            AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION.to_string();
        predecessor.claude_model_admission = AgentTokenClaudeModelAdmission::AsShipped;
        validate_agent_token_campaign_source(&predecessor)
            .expect(".46 as-shipped Claude evidence remains readable");

        predecessor.protocol_revision =
            AGENT_TOKEN_STRICT_ONLY_PREDECESSOR_PROTOCOL_REVISION.to_string();
        assert!(validate_agent_token_campaign_source(&predecessor)
            .unwrap_err()
            .contains("as_shipped"));
    }

    #[test]
    fn protocol_46_complete_replacement_manifest_remains_readable() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("fixture.json");
        fs::write(&fixture, "{}\n").unwrap();
        let mut manifest = validation_manifest(AgentTokenCampaignScope::Complete, 0, fixture);
        manifest.protocol_revision =
            AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION.to_string();
        manifest.runtime.executor = AgentTokenExecutor::Claude;
        manifest.runtime.claude_program = Some(PathBuf::from("claude"));
        manifest.runtime.executor_version = Some("2.1.235 (Claude Code)".to_string());
        manifest.model.provider = "anthropic".to_string();
        manifest.tool_policy = "claude_code_local_tools".to_string();
        manifest.claude_model_admission = AgentTokenClaudeModelAdmission::AsShipped;
        manifest.functional_replacement_policy =
            AgentTokenFunctionalReplacementPolicy::FirstValidUnacceptedLaneOnce;

        validate_agent_token_campaign_source(&manifest)
            .expect(".46 complete replacement evidence remains readable");
    }

    #[test]
    fn protocols_26_and_27_are_readable_as_frozen_200_session_predecessors_only() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("fixture.json");
        fs::write(&fixture, "{}\n").unwrap();
        let mut predecessor = validation_manifest(AgentTokenCampaignScope::Complete, 0, fixture);
        predecessor.protocol_revision =
            AGENT_TOKEN_200_SESSION_PREDECESSOR_PROTOCOL_REVISION.to_string();
        predecessor.attempts_per_cell = 20;

        validate_agent_token_campaign_source(&predecessor)
            .expect(".26 200-session evidence remains readable");
        let error = validate_agent_token_campaign(&predecessor)
            .expect_err(".26 must not start under the .28 binary");
        assert!(error.contains(AGENT_TOKEN_PROTOCOL_REVISION));

        predecessor.protocol_revision =
            AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION.to_string();
        validate_agent_token_campaign_source(&predecessor)
            .expect(".27 200-session evidence remains readable");
        let error = validate_agent_token_campaign(&predecessor)
            .expect_err(".27 must not start a new campaign under the .28 binary");
        assert!(error.contains(AGENT_TOKEN_PROTOCOL_REVISION));
    }

    #[test]
    fn cross_campaign_comparison_blocks_different_project_document_loading() {
        let fixture = PathBuf::from("fixture");
        let baseline_manifest =
            validation_manifest(AgentTokenCampaignScope::Smoke, 0, fixture.clone());
        let candidate_manifest =
            validation_manifest(AgentTokenCampaignScope::Smoke, 8_192, fixture);
        let baseline_schedule = build_agent_token_schedule(&baseline_manifest);
        let candidate_schedule = build_agent_token_schedule(&candidate_manifest);
        let baseline =
            build_agent_token_report(&baseline_manifest, &baseline_schedule, &[]).unwrap();
        let candidate =
            build_agent_token_report(&candidate_manifest, &candidate_schedule, &[]).unwrap();
        let comparison = compare_agent_token_reports(&baseline, &candidate);
        assert!(comparison
            .blockers
            .iter()
            .any(|blocker| blocker == "project-document loading differs"));
    }

    #[test]
    fn codex_usage_import_uses_provider_total_without_double_counting_cache() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("codex.jsonl");
        fs::write(
            &source,
            concat!(
                "{\"type\":\"thread.started\",\"thread_id\":\"T\"}\n",
                "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"cached_input_tokens\":60,\"cache_write_input_tokens\":0,\"output_tokens\":25,\"reasoning_output_tokens\":5}}\n"
            ),
        )
        .unwrap();
        let usage = import_codex_usage(
            &source,
            "run-1",
            "GD-01",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            &model(),
        )
        .unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.cached_input_tokens, Some(60));
        assert_eq!(usage.output_tokens, 25);
        assert_eq!(usage.reasoning_tokens, Some(5));
        assert_eq!(usage.provider_total_tokens, 125);
    }

    #[test]
    fn transcript_validator_requires_real_mode_commands_and_local_ait_closeout() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("codex.jsonl");
        let events = [
            "/bin/zsh -lc 'ait task start --title fix --intent fix --local'",
            "/bin/zsh -lc 'npm test'",
            "/bin/zsh -lc 'ait task finish LCT-1 --message fix --local'",
        ];
        let body = events
            .iter()
            .enumerate()
            .map(|(index, command)| {
                serde_json::json!({
                    "type": "item.completed",
                    "item": {
                        "id": format!("item-{index}"),
                        "type": "command_execution",
                        "command": command,
                    }
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&source, format!("{body}\n")).unwrap();
        let transcript = extract_and_validate_codex_transcript(
            &source,
            "run-1",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .unwrap();
        assert!(transcript.valid, "{:?}", transcript.errors);
    }

    #[test]
    fn sprint_on_transcript_requires_bound_task_start() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("codex.jsonl");
        write_test_transcript(
            &source,
            &[
                "/opt/ait task start --from docs/sprints/benchmark_task.md#agent-token-benchmark/run-1/implement --intent repair --local --json",
                "npm test",
                "/opt/ait task finish LCT-1 --message repair --local --json",
            ],
        );
        let sprint_on = extract_and_validate_codex_transcript_with_workflow_options(
            &source,
            "run-1",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            AgentTokenTranscriptWorkflowOptions {
                ait_sprint_mode: AgentTokenAitSprintMode::On,
                ait_edit_root_mode: None,
                git_worktree_mode: AgentTokenGitWorktreeMode::AgentManaged,
                clean_main_head_proven: false,
            },
        )
        .unwrap();
        assert!(sprint_on.valid, "{:?}", sprint_on.errors);

        let sprint_off = extract_and_validate_codex_transcript(
            &source,
            "run-1",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .unwrap();
        assert!(!sprint_off.valid);
        assert!(sprint_off
            .errors
            .iter()
            .any(|error| error.contains("sprint-off mode used forbidden")));
    }

    #[test]
    fn git_transcript_requires_complete_linked_worktree_lifecycle() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("codex.jsonl");
        write_test_transcript(
            &source,
            &[
                "/bin/zsh -lc '/usr/bin/git status --short'",
                "/bin/zsh -lc '/usr/bin/git rev-parse --git-dir'",
                "/bin/zsh -lc '/usr/bin/git worktree add -b benchmark-task /tmp/git-task-worktree main'",
                "/bin/zsh -lc 'cd /tmp/git-task-worktree && npm test && /usr/bin/git add --all && /usr/bin/git commit -m repair'",
                "/bin/zsh -lc 'cd /tmp/workspace && /usr/bin/git merge --ff-only benchmark-task && /usr/bin/git worktree remove /tmp/git-task-worktree && /usr/bin/git branch -d benchmark-task'",
            ],
        );

        let transcript = extract_and_validate_codex_transcript(
            &source,
            "run-1",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .unwrap();

        assert!(transcript.valid, "{:?}", transcript.errors);
        for required in [
            "git worktree add",
            "git commit",
            "git merge",
            "git worktree remove",
            "git branch --delete",
        ] {
            assert!(
                transcript
                    .observed_required_commands
                    .contains(&required.to_string()),
                "missing {required:?} in {:?}",
                transcript.observed_required_commands
            );
        }
    }

    #[test]
    fn git_transcript_is_valid_without_any_read_only_discovery() {
        // The declared worktree lifecycle already proves Git-mode fidelity.
        // Requiring a discovery invocation would charge Git an inspection cost
        // that the AIT branch never pays, so a lane that inspects nothing must
        // stay protocol-valid.
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("codex.jsonl");
        write_test_transcript(
            &source,
            &[
                "/bin/zsh -lc '/usr/bin/git worktree add -b benchmark-task /tmp/git-task-worktree main'",
                "/bin/zsh -lc 'cd /tmp/git-task-worktree && npm test && /usr/bin/git add --all && /usr/bin/git commit -m repair'",
                "/bin/zsh -lc 'cd /tmp/workspace && /usr/bin/git merge --ff-only benchmark-task && /usr/bin/git worktree remove /tmp/git-task-worktree && /usr/bin/git branch -d benchmark-task'",
            ],
        );

        let transcript = extract_and_validate_codex_transcript(
            &source,
            "run-1",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .unwrap();

        assert!(transcript.valid, "{:?}", transcript.errors);
        for absent in [
            "git status",
            "git diff",
            "git log",
            "git show",
            "git rev-parse",
        ] {
            assert!(
                !transcript
                    .observed_required_commands
                    .contains(&absent.to_string()),
                "unexpected {absent:?} in {:?}",
                transcript.observed_required_commands
            );
        }
    }

    #[test]
    fn app_equivalent_git_transcript_allows_only_optional_read_only_git() {
        let valid = validate_agent_token_command_list_with_workflow_options(
            vec![
                "git status --short".to_string(),
                "git diff --check".to_string(),
                "npm test".to_string(),
            ],
            "managed-run",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            AgentTokenTranscriptWorkflowOptions {
                ait_sprint_mode: AgentTokenAitSprintMode::Off,
                ait_edit_root_mode: None,
                git_worktree_mode: AgentTokenGitWorktreeMode::CodexAppEquivalentManaged,
                clean_main_head_proven: true,
            },
        )
        .unwrap();
        assert!(valid.valid, "{:?}", valid.errors);
        assert!(valid
            .observed_required_commands
            .iter()
            .any(|command| command == "read-only git status --short"));
        assert!(valid
            .observed_required_commands
            .iter()
            .all(|command| !command.contains("worktree add")));

        for mutation in [
            "git add --all",
            "git commit -m repair",
            "git worktree add --detach /tmp/other HEAD",
            "git checkout -- game.js",
        ] {
            let rejected = validate_agent_token_command_list_with_workflow_options(
                vec![mutation.to_string()],
                "managed-run",
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenAccountingProfile::SteadyStateTaskCost,
                AgentTokenTranscriptWorkflowOptions {
                    ait_sprint_mode: AgentTokenAitSprintMode::Off,
                    ait_edit_root_mode: None,
                    git_worktree_mode: AgentTokenGitWorktreeMode::CodexAppEquivalentManaged,
                    clean_main_head_proven: true,
                },
            )
            .unwrap();
            assert!(!rejected.valid, "mutation was admitted: {mutation}");
            assert!(rejected
                .errors
                .iter()
                .any(|error| error.contains("host-owned mutation")));
        }
    }

    #[test]
    fn returned_edit_root_treatment_rejects_task_start_edit_root_override() {
        let commands = vec![
            "/opt/ait task start --title repair --intent repair --local --json".to_string(),
            "npm test".to_string(),
            "/opt/ait task finish LCT-1 --message repair --local --json".to_string(),
        ];
        let returned = validate_agent_token_command_list_with_workflow_options(
            commands.clone(),
            "returned-run",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            AgentTokenTranscriptWorkflowOptions {
                ait_sprint_mode: AgentTokenAitSprintMode::Off,
                ait_edit_root_mode: Some(AgentTokenAitEditRootMode::Returned),
                git_worktree_mode: AgentTokenGitWorktreeMode::AgentManaged,
                clean_main_head_proven: false,
            },
        )
        .unwrap();
        assert!(returned.valid, "{:?}", returned.errors);
        assert!(returned
            .observed_required_commands
            .contains(&"ait task start without --edit-root".to_string()));

        let mut overridden_commands = commands;
        overridden_commands[0] = "/opt/ait task start --title repair --intent repair --edit-root /tmp/forced --local --json".to_string();
        let overridden = validate_agent_token_command_list_with_workflow_options(
            overridden_commands,
            "returned-run",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
            AgentTokenTranscriptWorkflowOptions {
                ait_sprint_mode: AgentTokenAitSprintMode::Off,
                ait_edit_root_mode: Some(AgentTokenAitEditRootMode::Returned),
                git_worktree_mode: AgentTokenGitWorktreeMode::AgentManaged,
                clean_main_head_proven: false,
            },
        )
        .unwrap();
        assert!(!overridden.valid);
        assert!(overridden
            .errors
            .iter()
            .any(|error| error.contains("forbidden --edit-root")));
    }

    #[test]
    fn git_transcript_rejects_incomplete_or_remote_worktree_lifecycle() {
        let cases = [
            (
                vec![
                    "git status --short",
                    "git add --all && git commit -m repair",
                    "npm test",
                    "git merge --ff-only benchmark-task",
                    "git worktree remove /tmp/git-task-worktree",
                    "git branch -d benchmark-task",
                ],
                "exactly one git worktree add; observed 0",
            ),
            (
                vec![
                    "git status --short",
                    "git worktree add -b benchmark-task /tmp/git-task-worktree main",
                    "npm test && git add --all && git commit -m repair",
                    "git merge benchmark-task",
                    "git worktree remove /tmp/git-task-worktree",
                    "git branch -d benchmark-task",
                ],
                "merge --ff-only",
            ),
            (
                vec![
                    "git status --short",
                    "git worktree add -b benchmark-task /tmp/git-task-worktree main",
                    "npm test && git add --all && git commit -m repair",
                    "git merge --ff-only benchmark-task",
                    "git branch -d benchmark-task",
                ],
                "exactly one git worktree remove; observed 0",
            ),
            (
                vec![
                    "git status --short",
                    "git worktree add -b benchmark-task /tmp/git-task-worktree main",
                    "npm test && git add --all && git commit -m repair",
                    "git merge --ff-only benchmark-task",
                    "git worktree remove /tmp/git-task-worktree",
                ],
                "exactly one git branch deletion; observed 0",
            ),
            (
                vec![
                    "git status --short",
                    "git worktree add -b benchmark-task /tmp/git-task-worktree main",
                    "npm test && git add --all && git commit -m first && git commit --allow-empty -m second",
                    "git merge --ff-only benchmark-task",
                    "git worktree remove /tmp/git-task-worktree",
                    "git branch -d benchmark-task",
                ],
                "exactly 1 git commit command(s); observed 2",
            ),
            (
                vec![
                    "git status --short",
                    "git worktree add -b benchmark-task /tmp/git-task-worktree main",
                    "npm test && git add --all && git commit -m repair",
                    "git push origin benchmark-task",
                    "git merge --ff-only benchmark-task",
                    "git worktree remove /tmp/git-task-worktree",
                    "git branch -d benchmark-task",
                ],
                "forbidden remote operation",
            ),
        ];

        for (index, (commands, expected)) in cases.iter().enumerate() {
            let temp = tempfile::tempdir().unwrap();
            let source = temp
                .path()
                .join(format!("codex-git-lifecycle-{index}.jsonl"));
            write_test_transcript(&source, commands);
            let transcript = extract_and_validate_codex_transcript(
                &source,
                "run-1",
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenAccountingProfile::SteadyStateTaskCost,
            )
            .unwrap();
            assert!(
                transcript
                    .errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "missing {expected:?}: {:?}",
                transcript.errors
            );
        }
    }

    #[test]
    fn ait_transcript_ignores_forbidden_spellings_inside_compliance_searches() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("codex.jsonl");
        write_test_transcript(
            &source,
            &[
                "/bin/zsh -lc '/opt/ait task start --title fix --intent fix --local'",
                "/bin/zsh -lc 'npm test'",
                r#"/bin/zsh -lc "rg -n \"ait-server| --remote|ait push|ait pull|ait remote|ait plan|ait workflow ready|ait queue summary|ait task list|ait change list|ait task audit|task start --from\" .""#,
                "/bin/zsh -lc 'rg ait-server . || true'",
                "/bin/zsh -lc 'echo /opt/ait push'",
                "/bin/zsh -lc '/opt/ait task finish LT-1 --message fix --local'",
            ],
        );

        let transcript = extract_and_validate_codex_transcript(
            &source,
            "run-1",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .unwrap();

        assert!(transcript.valid, "{:?}", transcript.errors);
        assert!(!command_invokes_ait_server("rg ait-server ."));
        assert!(!command_invokes_ait("echo /opt/ait push"));
    }

    #[test]
    fn ait_transcript_rejects_real_forbidden_surface_invocations() {
        let cases = [
            ("/bin/zsh -lc '/opt/ait-server serve'", "ait-server"),
            (
                r#"cmd.exe /C "C:\tools\ait-server.exe serve""#,
                "ait-server",
            ),
            ("/opt/ait push", "ait push"),
            (r"C:\tools\ait.exe pull", "ait pull"),
            ("ait remote list", "ait remote"),
            (
                r#"powershell.exe -Command "C:\tools\ait.exe plan list""#,
                "ait plan",
            ),
            ("/opt/ait workflow ready", "ait workflow ready"),
            (
                "/bin/zsh -lc '/opt/ait queue summary --json'",
                "ait queue summary",
            ),
            (r"C:\tools\ait.exe task list --all", "ait task list"),
            ("ait change list --all", "ait change list"),
            (
                r#"powershell.exe -Command "C:\tools\ait.exe task audit LT-1 --json""#,
                "ait task audit",
            ),
            ("/opt/ait snapshot list", "outside the measured lifecycle"),
            ("/opt/ait status --remote=origin", " --remote"),
            (
                "/opt/ait task start --from docs/sprint.md#item",
                "task start --from",
            ),
        ];
        for (index, (forbidden_command, expected)) in cases.iter().enumerate() {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join(format!("codex-{index}.jsonl"));
            write_test_transcript(
                &source,
                &[
                    "/opt/ait task start --title fix --intent fix --local",
                    forbidden_command,
                    "npm test",
                    "/opt/ait task finish LT-1 --message fix --local",
                ],
            );

            let transcript = extract_and_validate_codex_transcript(
                &source,
                "run-1",
                AgentTokenMode::AitLinearSingleSession,
                AgentTokenAccountingProfile::SteadyStateTaskCost,
            )
            .unwrap();

            assert!(
                transcript
                    .errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "missing {expected:?} rejection for {forbidden_command:?}: {:?}",
                transcript.errors
            );
        }
    }

    #[test]
    fn ait_transcript_requires_one_local_task_lifecycle() {
        let cases = [
            (
                vec![
                    "/opt/ait task start --title fix --intent fix",
                    "npm test",
                    "/opt/ait task finish LT-1 --message fix --local",
                ],
                "omitted --local from ait task start",
            ),
            (
                vec![
                    "/opt/ait task start --title fix --intent fix --local",
                    "/opt/ait task start --title retry --intent retry --local",
                    "npm test",
                    "/opt/ait task finish LT-1 --message fix --local",
                ],
                "exactly one ait task start; observed 2",
            ),
            (
                vec![
                    "/opt/ait task start --title fix --intent fix --local",
                    "npm test",
                    "/opt/ait task finish LT-1 --message fix",
                ],
                "omitted --local from ait task finish",
            ),
        ];
        for (index, (commands, expected)) in cases.iter().enumerate() {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join(format!("codex-lifecycle-{index}.jsonl"));
            write_test_transcript(&source, commands);

            let transcript = extract_and_validate_codex_transcript(
                &source,
                "run-1",
                AgentTokenMode::AitLinearSingleSession,
                AgentTokenAccountingProfile::SteadyStateTaskCost,
            )
            .unwrap();

            assert!(
                transcript
                    .errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "missing {expected:?}: {:?}",
                transcript.errors
            );
        }
    }

    #[test]
    fn first_use_ait_transcript_preserves_bootstrap_and_snapshot_requirements() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("codex.jsonl");
        write_test_transcript(
            &source,
            &[
                "/bin/zsh -lc '/opt/ait init --json'",
                "/bin/zsh -lc '/opt/ait config set workflow.mode solo_local --json'",
                "/bin/zsh -lc '/opt/ait snapshot create --message baseline --json'",
                "/bin/zsh -lc '/opt/ait task start --title fix --intent fix --local'",
                "/bin/zsh -lc 'npm test'",
                "/bin/zsh -lc '/opt/ait snapshot create --message checkpoint --json'",
                "/bin/zsh -lc '/opt/ait task finish LT-1 --message fix --local'",
            ],
        );

        let transcript = extract_and_validate_codex_transcript(
            &source,
            "run-1",
            AgentTokenMode::AitLinearSingleSession,
            AgentTokenAccountingProfile::FirstUseTotalCost,
        )
        .unwrap();

        assert!(transcript.valid, "{:?}", transcript.errors);
        for command in [
            "ait init",
            "ait config set",
            "ait task start",
            "ait task finish",
        ] {
            assert!(
                transcript
                    .observed_required_commands
                    .contains(&command.to_string()),
                "missing {command:?} in {:?}",
                transcript.observed_required_commands
            );
        }
    }

    #[test]
    fn git_transcript_rejects_candidate_metadata_context_override() {
        let forbidden_commands = [
            "GIT_DIR=/tmp/copied-git git status --short",
            "env GIT_WORK_TREE=/tmp/copied-worktree git status --short",
            "git --git-dir /tmp/copied-git status --short",
            "git --work-tree=/tmp/copied-worktree status --short",
            "export GIT_DIR=/tmp/copied-git; git status --short",
        ];
        for (index, forbidden_command) in forbidden_commands.iter().enumerate() {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join(format!("codex-metadata-{index}.jsonl"));
            write_test_transcript(
                &source,
                &[
                    forbidden_command,
                    "git worktree add -b benchmark-task /tmp/git-task-worktree main",
                    "npm test && git add --all && git commit -m repair",
                    "git merge --ff-only benchmark-task",
                    "git worktree remove /tmp/git-task-worktree",
                    "git branch -d benchmark-task",
                ],
            );

            let transcript = extract_and_validate_codex_transcript(
                &source,
                "run-1",
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenAccountingProfile::SteadyStateTaskCost,
            )
            .unwrap();
            assert!(!transcript.valid);
            assert!(
                transcript.errors.iter().any(|error| error
                    .contains("overrode the runner-owned isolated repository metadata context")),
                "missing metadata override rejection for {forbidden_command:?}: {:?}",
                transcript.errors
            );
        }
    }

    #[test]
    fn first_use_git_transcript_requires_repository_local_identity_pin() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("codex.jsonl");
        let events = [
            "/bin/zsh -lc 'git init --initial-branch=main'",
            "/bin/zsh -lc 'git status --short'",
            "/bin/zsh -lc 'git add --all && git commit -m baseline'",
            "/bin/zsh -lc 'npm test'",
            "/bin/zsh -lc 'git add --all && git commit -m repair'",
        ];
        let body = events
            .iter()
            .enumerate()
            .map(|(index, command)| {
                serde_json::json!({
                    "type": "item.completed",
                    "item": {
                        "id": format!("item-{index}"),
                        "type": "command_execution",
                        "command": command,
                    }
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&source, format!("{body}\n")).unwrap();

        let transcript = extract_and_validate_codex_transcript(
            &source,
            "run-1",
            AgentTokenMode::GitLinearSingleSession,
            AgentTokenAccountingProfile::FirstUseTotalCost,
        )
        .unwrap();
        assert!(!transcript.valid);
        assert!(transcript
            .errors
            .iter()
            .any(|error| error.contains("git config user.name")));
        assert!(transcript
            .errors
            .iter()
            .any(|error| error.contains("git config user.email")));
    }

    #[test]
    fn schedule_is_seeded_block_randomized_adjacent_and_exact() {
        let manifest = AgentTokenCampaignManifest {
            contract: AGENT_TOKEN_CAMPAIGN_CONTRACT.to_string(),
            campaign_id: "smoke".to_string(),
            protocol_revision: AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
            campaign_scope: AgentTokenCampaignScope::Smoke,
            accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
            ait_sprint_mode: AgentTokenAitSprintMode::Off,
            ait_edit_root_mode: crate::agent_token::AgentTokenAitEditRootMode::Explicit,
            git_worktree_mode: AgentTokenGitWorktreeMode::AgentManaged,
            claude_model_admission: crate::agent_token::AgentTokenClaudeModelAdmission::Strict,
            functional_replacement_policy: AgentTokenFunctionalReplacementPolicy::None,
            seed: 42,
            attempts_per_cell: 2,
            workload_ids: vec!["GD-01".to_string(), "GD-02".to_string()],
            modes: vec![
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ],
            model: model(),
            runtime: AgentTokenRuntime {
                executor: AgentTokenExecutor::default(),
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
                fixture_manifest: PathBuf::from("fixture"),
                run_timeout_seconds: 60,
                ait_first_use_worktree_add_dir: None,
                project_doc_max_bytes: 0,
            },
            cache_class: "provider_default".to_string(),
            network_policy: "disabled_except_loopback".to_string(),
            tool_policy: "codex_shell_only".to_string(),
            bootstrap_resamples: 1_000,
            limitations: Vec::new(),
        };
        let first = build_agent_token_schedule(&manifest);
        let second = build_agent_token_schedule(&manifest);
        assert_eq!(first.entry_count, 8);
        assert_eq!(first.entries[0].run_id, second.entries[0].run_id);
        assert_eq!(
            first
                .entries
                .iter()
                .map(|entry| entry.run_id.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            8
        );
        for pair in first.entries.chunks_exact(2) {
            assert_eq!(pair[0].attempt, pair[1].attempt);
            assert_eq!(pair[0].workload_id, pair[1].workload_id);
            assert_eq!(pair[1].randomized_order, pair[0].randomized_order + 1);
            assert_eq!(
                pair.iter().map(|entry| entry.mode).collect::<BTreeSet<_>>(),
                BTreeSet::from([
                    AgentTokenMode::GitLinearSingleSession,
                    AgentTokenMode::AitLinearSingleSession,
                ])
            );
        }
        assert_eq!(
            first
                .entries
                .chunks_exact(2)
                .map(|pair| pair[0].mode)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ])
        );
        assert!(first.entries.chunks_exact(4).all(|block| {
            block[0].attempt == block[3].attempt
                && block[0].randomized_order == 1
                && block[3].randomized_order == 4
        }));
    }

    fn synthetic_run(
        entry: &AgentTokenScheduleEntry,
        accepted: bool,
        tokens: u64,
    ) -> AgentTokenRunSummary {
        AgentTokenRunSummary {
            contract: AGENT_TOKEN_RUN_SUMMARY_CONTRACT.to_string(),
            campaign_id: "smoke".to_string(),
            run_id: entry.run_id.clone(),
            workload_id: entry.workload_id.clone(),
            mode: entry.mode,
            accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
            attempt: entry.attempt,
            block_index: entry.block_index,
            randomized_order: entry.randomized_order,
            initial_content_digest: "sha256:initial".to_string(),
            final_content_digest: Some("sha256:final".to_string()),
            codex_exit_code: Some(0),
            codex_timed_out: false,
            elapsed_ms: 1,
            infrastructure_failure: None,
            provider_refusal: false,
            provider_stop_reason: None,
            usage: Some(NormalizedAgentTokenUsage {
                contract: AGENT_TOKEN_USAGE_CONTRACT.to_string(),
                run_id: entry.run_id.clone(),
                workload_id: entry.workload_id.clone(),
                mode: entry.mode,
                accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
                model_provider: "openai".to_string(),
                model_id: "gpt-test".to_string(),
                model_revision: "test-revision".to_string(),
                reasoning_effort: "medium".to_string(),
                input_tokens: tokens - 10,
                cached_input_tokens: Some(0),
                cache_write_input_tokens: Some(0),
                output_tokens: 10,
                reasoning_tokens: Some(2),
                provider_total_tokens: tokens,
                completed_turns: 1,
                usage_provenance: "test".to_string(),
            }),
            transcript: AgentTokenCommandTranscript {
                contract: AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
                run_id: entry.run_id.clone(),
                mode: entry.mode,
                accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
                command_count: 1,
                commands: vec!["test".to_string()],
                valid: true,
                errors: Vec::new(),
                observed_required_commands: Vec::new(),
            },
            secondary_metrics: AgentTokenSecondaryMetrics::default(),
            evaluator_exit_code: Some(if accepted { 0 } else { 1 }),
            evaluator_score: Some(if accepted { 100 } else { 80 }),
            evaluator_accepted: accepted,
            browser: AgentTokenBrowserReport {
                contract: AGENT_TOKEN_BROWSER_REPORT_CONTRACT.to_string(),
                workload_id: entry.workload_id.clone(),
                required_for_equivalent_completion: true,
                status: if accepted { "passed" } else { "failed" }.to_string(),
                desktop_passed: Some(accepted),
                mobile_passed: Some(accepted),
                console_errors: Some(0),
                failed_requests: Some(0),
                horizontal_overflow: Some(false),
                notes: Vec::new(),
            },
            workflow_closed: true,
            valid_attempt: true,
            accepted_equivalent: accepted,
            invalid_reasons: Vec::new(),
            failure_reasons: if accepted {
                Vec::new()
            } else {
                vec!["candidate failed".to_string()]
            },
        }
    }

    #[test]
    fn legacy_run_adjudication_is_digest_linked_narrow_and_raw_immutable() {
        let temp = tempfile::tempdir().unwrap();
        let campaign_dir = temp.path();
        let entry = AgentTokenScheduleEntry {
            run_id: "legacy-git-run".to_string(),
            workload_id: "GD-04".to_string(),
            mode: AgentTokenMode::GitLinearSingleSession,
            attempt: 8,
            block_index: 8,
            randomized_order: 2,
        };
        let run_dir = campaign_dir.join("runs").join(&entry.run_id);
        fs::create_dir_all(run_dir.join("private")).unwrap();
        let raw_events = run_dir.join("private/codex-events.raw.jsonl");
        write_test_transcript(
            &raw_events,
            &[
                "/usr/bin/git status --short",
                "/usr/bin/git rev-parse --git-dir",
                "/usr/bin/git worktree add -b benchmark-task /tmp/git-task-worktree main",
                "cd /tmp/git-task-worktree && npm test && /usr/bin/git add --all && /usr/bin/git commit -m repair",
                "/usr/bin/git merge --ff-only benchmark-task && /usr/bin/git worktree remove /tmp/git-task-worktree && /usr/bin/git branch -d benchmark-task",
            ],
        );
        let valid_transcript = extract_and_validate_codex_transcript(
            &raw_events,
            &entry.run_id,
            entry.mode,
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .unwrap();
        assert!(valid_transcript.valid, "{:?}", valid_transcript.errors);

        let mut source = synthetic_run(&entry, true, 100);
        source.transcript = valid_transcript;
        source.transcript.valid = false;
        source.transcript.errors = vec![GIT_METADATA_CONTEXT_OVERRIDE_ERROR.to_string()];
        source.valid_attempt = false;
        source.accepted_equivalent = false;
        source.invalid_reasons = vec![GIT_METADATA_CONTEXT_OVERRIDE_ERROR.to_string()];
        let source_path = run_dir.join("run-summary.json");
        write_json_new(&source_path, &source).unwrap();
        let original_source_bytes = fs::read(&source_path).unwrap();

        let mut manifest = validation_manifest(
            AgentTokenCampaignScope::Smoke,
            0,
            campaign_dir.join("fixture.json"),
        );
        manifest.campaign_id = source.campaign_id.clone();
        manifest.protocol_revision = AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION.to_string();
        write_json_new(&campaign_dir.join("campaign-manifest.json"), &manifest).unwrap();

        let adjudication = build_agent_token_run_adjudication(
            campaign_dir,
            &source,
            AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION,
        )
        .unwrap();
        assert_eq!(
            adjudication.source_run_summary_sha256,
            sha256_digest(&original_source_bytes)
        );
        assert!(adjudication.effective_summary.valid_attempt);
        assert!(adjudication.effective_summary.accepted_equivalent);
        assert!(adjudication.effective_summary.transcript.valid);
        assert!(adjudication.effective_summary.transcript.errors.is_empty());
        write_json_new(
            &campaign_dir
                .join("adjudications")
                .join(format!("{}.json", entry.run_id)),
            &adjudication,
        )
        .unwrap();

        let raw = load_agent_token_raw_run_summaries(campaign_dir).unwrap();
        let effective = load_agent_token_run_summaries(campaign_dir).unwrap();
        assert!(!raw[0].valid_attempt);
        assert!(effective[0].valid_attempt);
        assert!(effective[0].accepted_equivalent);
        assert_eq!(fs::read(&source_path).unwrap(), original_source_bytes);

        let mut tampered = adjudication;
        tampered.reason.push_str(" tampered");
        fs::write(
            campaign_dir
                .join("adjudications")
                .join(format!("{}.json", entry.run_id)),
            serde_json::to_vec_pretty(&tampered).unwrap(),
        )
        .unwrap();
        let error = load_agent_token_run_summaries(campaign_dir).unwrap_err();
        assert!(
            error.contains("differs from its source evidence"),
            "{error}"
        );
        assert_eq!(fs::read(&source_path).unwrap(), original_source_bytes);
    }

    #[test]
    fn evidence_loader_preserves_source_scope_across_admitted_revisions() {
        let temp = tempfile::tempdir().unwrap();
        let manifest_path = temp.path().join("campaign.json");
        let fixture_path = temp.path().join("fixture.json");
        fs::write(&fixture_path, "{}\n").unwrap();
        let mut manifest = validation_manifest(AgentTokenCampaignScope::Complete, 0, fixture_path);
        manifest.protocol_revision = AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION.to_string();
        let mut legacy_value = serde_json::to_value(&manifest).unwrap();
        legacy_value["campaign_scope"] = serde_json::json!("pilot");
        legacy_value["attempts_per_cell"] = serde_json::json!(10);
        write_json_new(&manifest_path, &legacy_value).unwrap();
        assert!(load_agent_token_campaign(&manifest_path).is_err());
        let loaded = load_agent_token_campaign_for_evidence(&manifest_path).unwrap();
        assert_eq!(
            loaded.protocol_revision,
            AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION
        );
        assert_eq!(loaded.campaign_scope, AgentTokenCampaignScope::Pilot);

        let mut qualification = validation_manifest(
            AgentTokenCampaignScope::Qualification,
            0,
            temp.path().join("fixture.json"),
        );
        qualification.protocol_revision =
            AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION.to_string();
        let qualification_path = temp.path().join("qualification.json");
        write_json_new(&qualification_path, &qualification).unwrap();
        let loaded = load_agent_token_campaign_for_evidence(&qualification_path).unwrap();
        assert_eq!(
            loaded.campaign_scope,
            AgentTokenCampaignScope::Qualification
        );

        for (index, revision) in AGENT_TOKEN_COMPLETE_PREDECESSOR_PROTOCOL_REVISIONS
            .iter()
            .enumerate()
        {
            let mut predecessor = validation_manifest(
                AgentTokenCampaignScope::Complete,
                0,
                temp.path().join("fixture.json"),
            );
            predecessor.protocol_revision = (*revision).to_string();
            predecessor.attempts_per_cell = if *revision
                == AGENT_TOKEN_MANAGED_WORKTREE_PREFLIGHT_PREDECESSOR_PROTOCOL_REVISION
                || *revision == AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION
                || *revision == AGENT_TOKEN_MODEL_PURITY_PREDECESSOR_PROTOCOL_REVISION
                || *revision == AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION
                || *revision == AGENT_TOKEN_PROMPTED_INSPECTION_PREDECESSOR_PROTOCOL_REVISION
                || *revision == AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION
                || *revision == AGENT_TOKEN_200_SESSION_PREDECESSOR_PROTOCOL_REVISION
            {
                20
            } else {
                AGENT_TOKEN_PREDECESSOR_COMPLETE_ATTEMPTS_PER_WORKLOAD
            };
            let predecessor_path = temp.path().join(format!("predecessor-{index}.json"));
            write_json_new(&predecessor_path, &predecessor).unwrap();
            let loaded = load_agent_token_campaign_for_evidence(&predecessor_path).unwrap();
            assert_eq!(loaded.campaign_scope, AgentTokenCampaignScope::Complete);
        }

        manifest.protocol_revision = "game-development-2026-08-24.19".to_string();
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        assert!(load_agent_token_campaign_for_evidence(&manifest_path).is_err());
    }

    fn report_test_manifest(attempts_per_cell: usize) -> AgentTokenCampaignManifest {
        AgentTokenCampaignManifest {
            contract: AGENT_TOKEN_CAMPAIGN_CONTRACT.to_string(),
            campaign_id: "smoke".to_string(),
            protocol_revision: AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
            campaign_scope: AgentTokenCampaignScope::Smoke,
            accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
            ait_sprint_mode: AgentTokenAitSprintMode::Off,
            ait_edit_root_mode: crate::agent_token::AgentTokenAitEditRootMode::Explicit,
            git_worktree_mode: AgentTokenGitWorktreeMode::AgentManaged,
            claude_model_admission: crate::agent_token::AgentTokenClaudeModelAdmission::Strict,
            functional_replacement_policy: AgentTokenFunctionalReplacementPolicy::None,
            seed: 42,
            attempts_per_cell,
            workload_ids: vec!["GD-01".to_string()],
            modes: vec![
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ],
            model: model(),
            runtime: AgentTokenRuntime {
                executor: AgentTokenExecutor::default(),
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
                fixture_manifest: PathBuf::from("fixture"),
                run_timeout_seconds: 60,
                ait_first_use_worktree_add_dir: None,
                project_doc_max_bytes: 0,
            },
            cache_class: "provider_default".to_string(),
            network_policy: "disabled_except_loopback".to_string(),
            tool_policy: "codex_shell_only".to_string(),
            bootstrap_resamples: 1_000,
            limitations: Vec::new(),
        }
    }

    #[test]
    fn complete_two_hundred_session_campaign_is_publication_eligible() {
        let mut manifest = validation_manifest(
            AgentTokenCampaignScope::Complete,
            0,
            PathBuf::from("fixture"),
        );
        manifest.campaign_id = "smoke".to_string();
        let schedule = build_agent_token_schedule(&manifest);
        assert_eq!(schedule.entry_count, AGENT_TOKEN_COMPLETE_SCHEDULED_RUNS);
        let runs = schedule
            .entries
            .iter()
            .map(|entry| {
                synthetic_run(
                    entry,
                    true,
                    match entry.mode {
                        AgentTokenMode::GitLinearSingleSession => 100,
                        AgentTokenMode::AitLinearSingleSession => 50,
                    },
                )
            })
            .collect::<Vec<_>>();

        let report = build_agent_token_report(&manifest, &schedule, &runs).unwrap();
        assert_eq!(report.campaign_scope, "complete");
        assert_eq!(
            report.observed_run_count,
            AGENT_TOKEN_COMPLETE_SCHEDULED_RUNS
        );
        assert!(report.claim_eligible, "{:?}", report.blockers);
        assert!(report.source_protocol_claim_eligible);
        assert_eq!(
            report.current_policy_revision,
            AGENT_TOKEN_PROTOCOL_REVISION
        );
        assert_eq!(report.current_policy_evaluation_mode, "prospective");
        assert!(report.current_policy_criteria_met);
        assert!(report.current_policy_blockers.is_empty());
        assert!(report.blockers.is_empty());
        assert_eq!(report.aggregate_median_token_savings_percent, Some(50.0));
        assert_eq!(
            report.aggregate_token_savings_bootstrap_ci95,
            Some([50.0, 50.0])
        );

        let prefix = runs.iter().take(10).cloned().collect::<Vec<_>>();
        let prefix_report = build_agent_token_report(&manifest, &schedule, &prefix).unwrap();
        assert_eq!(prefix_report.observed_run_count, 10);
        assert!(!prefix_report.claim_eligible);
        assert!(!prefix_report.current_policy_criteria_met);
        assert!(prefix_report
            .blockers
            .iter()
            .any(|blocker| blocker == "The required accepted paired schedule is incomplete"));

        let mut incomplete = runs;
        let first_ait = schedule
            .entries
            .iter()
            .position(|entry| {
                entry.workload_id == "GD-01" && entry.mode == AgentTokenMode::AitLinearSingleSession
            })
            .unwrap();
        incomplete[first_ait] = synthetic_run(&schedule.entries[first_ait], false, 100);
        let blocked = build_agent_token_report(&manifest, &schedule, &incomplete).unwrap();
        assert!(!blocked.claim_eligible);
        assert!(!blocked.source_protocol_claim_eligible);
        assert!(!blocked.current_policy_criteria_met);
        assert!(blocked
            .blockers
            .iter()
            .any(|blocker| blocker == "The required accepted paired schedule is incomplete"));
        assert_eq!(
            blocked
                .comparisons
                .iter()
                .find(|comparison| comparison.workload_id == "GD-01")
                .unwrap()
                .acceptance_rate_deficit_percentage_points,
            5.0
        );
        assert!(
            !blocked
                .blockers
                .iter()
                .any(|blocker| blocker
                    == "AIT acceptance-rate deficit exceeds five percentage points")
        );

        let second_ait = schedule
            .entries
            .iter()
            .enumerate()
            .find_map(|(index, entry)| {
                (index != first_ait
                    && entry.workload_id == "GD-01"
                    && entry.mode == AgentTokenMode::AitLinearSingleSession)
                    .then_some(index)
            })
            .unwrap();
        incomplete[second_ait] = synthetic_run(&schedule.entries[second_ait], false, 100);
        let over_boundary = build_agent_token_report(&manifest, &schedule, &incomplete).unwrap();
        assert_eq!(
            over_boundary
                .comparisons
                .iter()
                .find(|comparison| comparison.workload_id == "GD-01")
                .unwrap()
                .acceptance_rate_deficit_percentage_points,
            10.0
        );
        assert!(
            over_boundary
                .blockers
                .iter()
                .any(|blocker| blocker
                    == "AIT acceptance-rate deficit exceeds five percentage points")
        );
    }

    #[test]
    fn legacy_pilot_remains_source_ineligible_but_gets_separate_retrospective_assessment() {
        let mut manifest =
            validation_manifest(AgentTokenCampaignScope::Pilot, 0, PathBuf::from("fixture"));
        manifest.campaign_id = "legacy-pilot".to_string();
        manifest.protocol_revision = AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION.to_string();
        let schedule = build_agent_token_schedule(&manifest);
        let runs = schedule
            .entries
            .iter()
            .map(|entry| {
                synthetic_run(
                    entry,
                    true,
                    match entry.mode {
                        AgentTokenMode::GitLinearSingleSession => 100,
                        AgentTokenMode::AitLinearSingleSession => 50,
                    },
                )
            })
            .collect::<Vec<_>>();

        let report = build_agent_token_report(&manifest, &schedule, &runs).unwrap();
        assert_eq!(
            report.protocol_revision,
            AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION
        );
        assert_eq!(report.campaign_scope, "pilot");
        assert!(!report.source_protocol_claim_eligible);
        assert!(!report.claim_eligible);
        assert!(report
            .blockers
            .iter()
            .any(|blocker| blocker == "pilot evidence is never claim eligible"));
        assert_eq!(
            report.current_policy_revision,
            AGENT_TOKEN_PROTOCOL_REVISION
        );
        assert_eq!(report.current_policy_evaluation_mode, "retrospective");
        assert!(!report.current_policy_criteria_met);
        assert!(report
            .current_policy_blockers
            .iter()
            .any(|blocker| blocker.contains("twenty-pair, 200-session matrix")));

        let markdown = render_agent_token_report_markdown(&report);
        assert!(markdown.contains("Source campaign scope: `pilot`"));
        assert!(markdown.contains("Source-protocol claim eligible: `false`"));
        assert!(markdown.contains("Current-policy criteria met: `false`"));
        assert!(markdown.contains("pilot evidence is never claim eligible"));
    }

    #[test]
    fn mixed_workflow_metrics_retain_asymmetric_patch_pairs() {
        let manifest = report_test_manifest(4);
        let schedule = build_agent_token_schedule(&manifest);
        let runs = schedule
            .entries
            .iter()
            .map(|entry| {
                let mut run = synthetic_run(
                    entry,
                    true,
                    match entry.mode {
                        AgentTokenMode::GitLinearSingleSession => 100,
                        AgentTokenMode::AitLinearSingleSession => 50,
                    },
                );
                match entry.mode {
                    AgentTokenMode::GitLinearSingleSession => {
                        run.elapsed_ms = 200;
                        run.secondary_metrics.file_change_items = 3;
                        run.secondary_metrics.apply_patch_rejected_attempts = 1;
                        run.secondary_metrics.apply_patch_attempts = 4;
                    }
                    AgentTokenMode::AitLinearSingleSession => {
                        run.elapsed_ms = 150;
                        run.secondary_metrics.file_change_items = 1;
                        run.secondary_metrics.apply_patch_rejected_attempts = 0;
                        run.secondary_metrics.apply_patch_attempts = 1;
                    }
                };
                run
            })
            .collect::<Vec<_>>();

        let report = build_agent_token_report(&manifest, &schedule, &runs).unwrap();
        let comparison = &report.comparisons[0];
        assert_eq!(report.contract, "ait-agent-token-benchmark-report/v3");
        assert_eq!(
            report.pair_admission_policy,
            AGENT_TOKEN_PAIR_ADMISSION_POLICY
        );
        assert_eq!(comparison.paired_valid_attempt_count, 4);
        assert_eq!(comparison.git_effective_tokens, Some(100.0));
        assert_eq!(comparison.ait_effective_tokens, Some(50.0));
        assert_eq!(comparison.token_savings_percent, Some(50.0));
        assert_eq!(comparison.git_effective_elapsed_ms, Some(200.0));
        assert_eq!(comparison.ait_effective_elapsed_ms, Some(150.0));
        assert_eq!(comparison.elapsed_savings_percent, Some(25.0));
        assert_eq!(
            comparison.git_effective_completed_file_change_items,
            Some(3.0)
        );
        assert_eq!(
            comparison.ait_effective_completed_file_change_items,
            Some(1.0)
        );
        assert_eq!(
            comparison.completed_file_change_reduction_percent,
            Some(100.0 * (1.0 - 1.0 / 3.0))
        );
        assert_eq!(
            comparison.git_effective_rejected_apply_patch_attempts,
            Some(1.0)
        );
        assert_eq!(
            comparison.ait_effective_rejected_apply_patch_attempts,
            Some(0.0)
        );
        assert_eq!(
            comparison.rejected_apply_patch_reduction_percent,
            Some(100.0)
        );
        assert_eq!(comparison.git_effective_apply_patch_attempts, Some(4.0));
        assert_eq!(comparison.ait_effective_apply_patch_attempts, Some(1.0));
        assert_eq!(comparison.apply_patch_attempt_reduction_percent, Some(75.0));
        assert_eq!(report.aggregate_median_elapsed_savings_percent, Some(25.0));
        assert_eq!(
            report.aggregate_median_apply_patch_attempt_reduction_percent,
            Some(75.0)
        );
        assert!(report.groups.iter().all(|group| {
            group.elapsed_ms_distribution.is_some()
                && group.completed_file_change_item_distribution.is_some()
                && group.rejected_apply_patch_attempt_distribution.is_some()
                && group.apply_patch_attempt_distribution.is_some()
        }));

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["contract"], "ait-agent-token-benchmark-report/v3");
        assert_eq!(
            json["pair_admission_policy"],
            AGENT_TOKEN_PAIR_ADMISSION_POLICY
        );
        assert_eq!(
            json["comparisons"][0]["git_effective_apply_patch_attempts"],
            4.0
        );
        assert_eq!(
            json["comparisons"][0]["ait_effective_apply_patch_attempts"],
            1.0
        );
        assert!(json.get("apply_patch_pair_threshold").is_none());

        let markdown = render_agent_token_report_markdown(&report);
        assert!(markdown.contains("## AIT vs Git Tokens"));
        assert!(markdown.contains("## Workflow Efficiency"));
        assert!(markdown.contains("File changes Git/AIT"));
        assert!(markdown.contains("Rejected patches Git/AIT"));
        assert!(markdown.contains("Total patches Git/AIT"));
        assert!(!markdown.contains("Patch-parity"));
    }

    #[test]
    fn legacy_report_remains_readable_and_is_blocked_by_protocol_revision() {
        let manifest = report_test_manifest(1);
        let schedule = build_agent_token_schedule(&manifest);
        let runs = schedule
            .entries
            .iter()
            .map(|entry| synthetic_run(entry, true, 100))
            .collect::<Vec<_>>();
        let current = build_agent_token_report(&manifest, &schedule, &runs).unwrap();
        let mut legacy_json = serde_json::to_value(&current).unwrap();
        let legacy = legacy_json.as_object_mut().unwrap();
        legacy.insert(
            "contract".to_string(),
            serde_json::json!("ait-agent-token-benchmark-report/v1"),
        );
        legacy.insert(
            "protocol_revision".to_string(),
            serde_json::json!("game-development-2026-08-23.13"),
        );
        for field in [
            "pair_admission_policy",
            "aggregate_median_elapsed_savings_percent",
            "aggregate_median_completed_file_change_reduction_percent",
            "aggregate_median_rejected_apply_patch_reduction_percent",
            "aggregate_median_apply_patch_attempt_reduction_percent",
            "source_protocol_claim_eligible",
            "current_policy_revision",
            "current_policy_evaluation_mode",
            "current_policy_criteria_met",
            "current_policy_blockers",
        ] {
            legacy.remove(field);
        }
        for group in legacy["groups"].as_array_mut().unwrap() {
            let group = group.as_object_mut().unwrap();
            for field in [
                "elapsed_ms_distribution",
                "completed_file_change_item_distribution",
                "rejected_apply_patch_attempt_distribution",
                "apply_patch_attempt_distribution",
            ] {
                group.remove(field);
            }
        }
        for comparison in legacy["comparisons"].as_array_mut().unwrap() {
            let comparison = comparison.as_object_mut().unwrap();
            for field in [
                "git_effective_elapsed_ms",
                "ait_effective_elapsed_ms",
                "elapsed_savings_percent",
                "git_effective_completed_file_change_items",
                "ait_effective_completed_file_change_items",
                "completed_file_change_reduction_percent",
                "git_effective_rejected_apply_patch_attempts",
                "ait_effective_rejected_apply_patch_attempts",
                "rejected_apply_patch_reduction_percent",
                "git_effective_apply_patch_attempts",
                "ait_effective_apply_patch_attempts",
                "apply_patch_attempt_reduction_percent",
            ] {
                comparison.remove(field);
            }
            comparison.insert(
                "paired_patch_parity_excluded_count".to_string(),
                serde_json::json!(0),
            );
            comparison.insert(
                "paired_patch_parity_excluded_attempts".to_string(),
                serde_json::json!([]),
            );
        }

        let legacy: AgentTokenReport = serde_json::from_value(legacy_json).unwrap();
        assert!(legacy.pair_admission_policy.is_empty());
        assert_eq!(legacy.aggregate_median_elapsed_savings_percent, None);
        assert!(legacy.groups[0].elapsed_ms_distribution.is_none());
        assert_eq!(legacy.comparisons[0].git_effective_elapsed_ms, None);
        let comparison = compare_agent_token_reports(&legacy, &current);
        assert!(!comparison.comparable);
        assert!(comparison
            .blockers
            .iter()
            .any(|blocker| blocker == "protocol revision differs"));
    }

    #[test]
    fn missing_counterpart_is_incomplete_without_workflow_metric_exclusion() {
        let manifest = report_test_manifest(2);
        let schedule = build_agent_token_schedule(&manifest);
        let runs = schedule
            .entries
            .iter()
            .filter(|entry| {
                !(entry.attempt == 2 && entry.mode == AgentTokenMode::AitLinearSingleSession)
            })
            .map(|entry| synthetic_run(entry, true, 100))
            .collect::<Vec<_>>();

        let report = build_agent_token_report(&manifest, &schedule, &runs).unwrap();
        let comparison = &report.comparisons[0];
        assert_eq!(comparison.paired_valid_attempt_count, 1);
        assert!(report
            .blockers
            .iter()
            .any(|blocker| blocker == "The required accepted paired schedule is incomplete"));
    }

    #[test]
    fn zero_git_workflow_baseline_retains_raw_values_without_reduction() {
        let manifest = report_test_manifest(1);
        let schedule = build_agent_token_schedule(&manifest);
        let runs = schedule
            .entries
            .iter()
            .map(|entry| {
                let mut run = synthetic_run(entry, true, 100);
                run.secondary_metrics.apply_patch_rejected_attempts =
                    if entry.mode == AgentTokenMode::AitLinearSingleSession {
                        1
                    } else {
                        0
                    };
                run.secondary_metrics.apply_patch_attempts =
                    run.secondary_metrics.apply_patch_rejected_attempts;
                run
            })
            .collect::<Vec<_>>();

        let report = build_agent_token_report(&manifest, &schedule, &runs).unwrap();
        let comparison = &report.comparisons[0];
        assert_eq!(
            comparison.git_effective_rejected_apply_patch_attempts,
            Some(0.0)
        );
        assert_eq!(
            comparison.ait_effective_rejected_apply_patch_attempts,
            Some(1.0)
        );
        assert_eq!(comparison.rejected_apply_patch_reduction_percent, None);
        assert_eq!(
            report.aggregate_median_rejected_apply_patch_reduction_percent,
            None
        );
    }

    #[test]
    fn report_counts_valid_failures_and_requires_bootstrap_claim_boundary() {
        let manifest = AgentTokenCampaignManifest {
            contract: AGENT_TOKEN_CAMPAIGN_CONTRACT.to_string(),
            campaign_id: "smoke".to_string(),
            protocol_revision: AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
            campaign_scope: AgentTokenCampaignScope::Smoke,
            accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
            ait_sprint_mode: AgentTokenAitSprintMode::Off,
            ait_edit_root_mode: crate::agent_token::AgentTokenAitEditRootMode::Explicit,
            git_worktree_mode: AgentTokenGitWorktreeMode::AgentManaged,
            claude_model_admission: crate::agent_token::AgentTokenClaudeModelAdmission::Strict,
            functional_replacement_policy: AgentTokenFunctionalReplacementPolicy::None,
            seed: 42,
            attempts_per_cell: 3,
            workload_ids: vec!["GD-01".to_string()],
            modes: vec![
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ],
            model: model(),
            runtime: AgentTokenRuntime {
                executor: AgentTokenExecutor::default(),
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
                fixture_manifest: PathBuf::from("fixture"),
                run_timeout_seconds: 60,
                ait_first_use_worktree_add_dir: None,
                project_doc_max_bytes: 0,
            },
            cache_class: "provider_default".to_string(),
            network_policy: "disabled_except_loopback".to_string(),
            tool_policy: "codex_shell_only".to_string(),
            bootstrap_resamples: 1_000,
            limitations: Vec::new(),
        };
        let schedule = build_agent_token_schedule(&manifest);
        let runs = schedule
            .entries
            .iter()
            .map(|entry| {
                let mut run = match entry.mode {
                    AgentTokenMode::GitLinearSingleSession => {
                        synthetic_run(entry, entry.attempt != 3, 100)
                    }
                    AgentTokenMode::AitLinearSingleSession => synthetic_run(entry, true, 60),
                };
                match entry.mode {
                    AgentTokenMode::GitLinearSingleSession => {
                        run.elapsed_ms = 100;
                        run.secondary_metrics.file_change_items = 2;
                        run.secondary_metrics.apply_patch_rejected_attempts = 1;
                        run.secondary_metrics.apply_patch_attempts = 3;
                    }
                    AgentTokenMode::AitLinearSingleSession => {
                        run.elapsed_ms = 60;
                        run.secondary_metrics.file_change_items = 1;
                        run.secondary_metrics.apply_patch_attempts = 1;
                    }
                }
                run
            })
            .collect::<Vec<_>>();
        let report = build_agent_token_report(&manifest, &schedule, &runs).unwrap();
        let git = report
            .groups
            .iter()
            .find(|group| group.mode == "git_linear_single_session")
            .unwrap();
        assert_eq!(git.valid_count, 3);
        assert_eq!(git.accepted_count, 2);
        assert_eq!(git.total_valid_attempt_tokens, 300);
        assert_eq!(git.effective_tokens_per_accepted_task, Some(150.0));
        assert!((git.acceptance_rate - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(report.comparisons[0].token_savings_percent, Some(60.0));
        assert_eq!(report.comparisons[0].paired_valid_attempt_count, 3);
        assert_eq!(report.comparisons[0].git_effective_elapsed_ms, Some(150.0));
        assert_eq!(report.comparisons[0].ait_effective_elapsed_ms, Some(60.0));
        assert_eq!(report.comparisons[0].elapsed_savings_percent, Some(60.0));
        assert_eq!(
            report.comparisons[0].git_effective_completed_file_change_items,
            Some(3.0)
        );
        assert_eq!(
            report.comparisons[0].ait_effective_completed_file_change_items,
            Some(1.0)
        );
        assert_eq!(
            report.comparisons[0].git_effective_rejected_apply_patch_attempts,
            Some(1.5)
        );
        assert_eq!(
            report.comparisons[0].ait_effective_rejected_apply_patch_attempts,
            Some(0.0)
        );
        assert_eq!(
            report.comparisons[0].git_effective_apply_patch_attempts,
            Some(4.5)
        );
        assert_eq!(
            report.comparisons[0].ait_effective_apply_patch_attempts,
            Some(1.0)
        );
        assert!(report.comparisons[0].token_savings_bootstrap_ci95.is_some());
        assert_eq!(report.aggregate_median_token_savings_percent, Some(60.0));
        assert!(
            !report.claim_eligible,
            "smoke evidence never supports a claim"
        );
    }
}
