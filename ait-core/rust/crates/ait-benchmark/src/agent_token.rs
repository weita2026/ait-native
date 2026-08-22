use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::statistics::DeterministicRng;
use crate::{summarize_samples, DistributionSummary};

pub const AGENT_TOKEN_CAMPAIGN_CONTRACT: &str = "ait-agent-token-benchmark-campaign/v1";
pub const AGENT_TOKEN_SCHEDULE_CONTRACT: &str = "ait-agent-token-benchmark-schedule/v1";
pub const AGENT_TOKEN_USAGE_CONTRACT: &str = "ait-agent-token-provider-usage/v1";
pub const AGENT_TOKEN_TRANSCRIPT_CONTRACT: &str = "ait-agent-token-command-transcript/v1";
pub const AGENT_TOKEN_RUN_SUMMARY_CONTRACT: &str = "ait-agent-token-benchmark-run-summary/v1";
pub const AGENT_TOKEN_REPORT_CONTRACT: &str = "ait-agent-token-benchmark-report/v1";
pub const AGENT_TOKEN_ENVIRONMENT_CONTRACT: &str = "ait-agent-token-benchmark-environment/v1";
pub const AGENT_TOKEN_BROWSER_REPORT_CONTRACT: &str = "ait-agent-token-browser-report/v1";
pub const AGENT_TOKEN_PROTOCOL_REVISION: &str = "game-development-2026-08-22.10";

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
    Pilot,
    Qualification,
}

impl AgentTokenCampaignScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Smoke => "smoke",
            Self::Pilot => "pilot",
            Self::Qualification => "qualification",
        }
    }

    fn minimum_attempts(&self) -> usize {
        match self {
            Self::Smoke => 1,
            Self::Pilot => 10,
            Self::Qualification => 20,
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
    pub codex_program: PathBuf,
    pub ait_program: PathBuf,
    pub git_program: PathBuf,
    pub node_program: PathBuf,
    pub browser_program: Option<PathBuf>,
    pub fixture_manifest: PathBuf,
    pub run_timeout_seconds: u64,
    pub ait_first_use_worktree_add_dir: Option<PathBuf>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    pub sprint_mode: String,
    pub ait_server_connected: bool,
    pub network_policy: String,
    pub cache_class: String,
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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentTokenSecondaryMetrics {
    pub agent_turns: usize,
    pub model_calls: usize,
    pub command_tool_calls: usize,
    pub file_change_items: usize,
    pub tool_output_bytes: u64,
    pub project_validation_calls: usize,
    pub repository_query_calls: usize,
    pub repeated_repository_query_calls: usize,
    pub help_calls: usize,
    pub file_read_or_search_calls: usize,
    pub tool_calls_by_family: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenReport {
    pub contract: String,
    pub campaign_id: String,
    pub protocol_revision: String,
    pub campaign_scope: String,
    pub accounting_profile: String,
    pub model: AgentTokenModelPin,
    pub cache_class: String,
    pub network_policy: String,
    pub generated_at: String,
    pub scheduled_run_count: usize,
    pub observed_run_count: usize,
    pub invalid_run_count: usize,
    pub groups: Vec<AgentTokenGroupReport>,
    pub comparisons: Vec<AgentTokenModeComparison>,
    pub aggregate_median_token_savings_percent: Option<f64>,
    pub aggregate_token_savings_bootstrap_ci95: Option<[f64; 2]>,
    pub claim_eligible: bool,
    pub blockers: Vec<String>,
    pub limitations: Vec<String>,
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenModeComparison {
    pub workload_id: String,
    pub git_effective_tokens: Option<f64>,
    pub ait_effective_tokens: Option<f64>,
    pub token_savings_percent: Option<f64>,
    pub token_savings_bootstrap_ci95: Option<[f64; 2]>,
    pub paired_valid_attempt_count: usize,
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
    validate_agent_token_campaign(&manifest)?;
    Ok(manifest)
}

pub fn validate_agent_token_campaign(manifest: &AgentTokenCampaignManifest) -> Result<(), String> {
    if manifest.contract != AGENT_TOKEN_CAMPAIGN_CONTRACT {
        return Err(format!(
            "Agent-token campaign contract must be {AGENT_TOKEN_CAMPAIGN_CONTRACT}, got {}",
            manifest.contract
        ));
    }
    if manifest.protocol_revision != AGENT_TOKEN_PROTOCOL_REVISION {
        return Err(format!(
            "Agent-token protocol revision must be {AGENT_TOKEN_PROTOCOL_REVISION}, got {}",
            manifest.protocol_revision
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
    if manifest.attempts_per_cell < manifest.campaign_scope.minimum_attempts() {
        return Err(format!(
            "{} campaign requires at least {} attempts per cell",
            manifest.campaign_scope.as_str(),
            manifest.campaign_scope.minimum_attempts()
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
    if manifest.campaign_scope != AgentTokenCampaignScope::Smoke
        && workloads != BTreeSet::from(["GD-01", "GD-02", "GD-03", "GD-04", "GD-05"])
    {
        return Err(
            "Pilot and qualification campaigns must contain all five workloads".to_string(),
        );
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
    if manifest.tool_policy != "codex_shell_only" {
        return Err("tool_policy must be codex_shell_only".to_string());
    }
    if manifest.runtime.run_timeout_seconds < 60 {
        return Err("runtime.run_timeout_seconds must be at least 60".to_string());
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

pub fn build_agent_token_schedule(manifest: &AgentTokenCampaignManifest) -> AgentTokenSchedule {
    let mut entries = Vec::new();
    for attempt in 1..=manifest.attempts_per_cell {
        let mut block = Vec::new();
        for workload_id in &manifest.workload_ids {
            for mode in &manifest.modes {
                block.push(AgentTokenScheduleEntry {
                    run_id: format!(
                        "{}-b{attempt:03}-{}-{}",
                        manifest.campaign_id,
                        workload_id.to_ascii_lowercase(),
                        mode.short_name()
                    ),
                    workload_id: workload_id.clone(),
                    mode: *mode,
                    attempt,
                    block_index: attempt,
                    randomized_order: 0,
                });
            }
        }
        let mut generator =
            DeterministicRng::new(manifest.seed ^ (attempt as u64).wrapping_mul(0x9E37_79B9));
        generator.shuffle(&mut block);
        for (order, entry) in block.iter_mut().enumerate() {
            entry.randomized_order = order + 1;
        }
        entries.extend(block);
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

pub fn extract_and_validate_codex_transcript(
    source: &Path,
    run_id: &str,
    mode: AgentTokenMode,
    profile: AgentTokenAccountingProfile,
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

    let mut errors = Vec::new();
    let mut observed_required_commands = Vec::new();
    match mode {
        AgentTokenMode::GitLinearSingleSession => {
            let discovery = [
                "git status",
                "git diff",
                "git log",
                "git show",
                "git branch",
                "git rev-parse",
            ];
            for required in discovery {
                if commands.iter().any(|command| command.contains(required)) {
                    observed_required_commands.push(required.to_string());
                }
            }
            if observed_required_commands.is_empty() {
                errors.push("Git mode did not show explicit Git discovery behavior".to_string());
            }
            if commands.iter().any(|command| command_invokes_ait(command)) {
                errors.push("Git mode invoked AIT".to_string());
            }
            if commands.iter().any(|command| {
                ["GIT_DIR=", "GIT_WORK_TREE=", "--git-dir", "--work-tree"]
                    .iter()
                    .any(|forbidden| command.contains(forbidden))
            }) {
                errors.push(
                    "Git mode overrode the runner-owned isolated repository metadata context"
                        .to_string(),
                );
            }
            if !commands
                .iter()
                .any(|command| command.contains("git commit"))
            {
                errors.push("Git mode did not commit its accepted candidate state".to_string());
            } else {
                observed_required_commands.push("git commit".to_string());
            }
            match profile {
                AgentTokenAccountingProfile::SteadyStateTaskCost => {
                    if commands.iter().any(|command| command.contains("git init")) {
                        errors.push(
                            "Steady-state Git mode repeated first-use repository bootstrap"
                                .to_string(),
                        );
                    }
                }
                AgentTokenAccountingProfile::FirstUseTotalCost => {
                    if commands.iter().any(|command| command.contains("git init")) {
                        observed_required_commands.push("git init".to_string());
                    } else {
                        errors.push("First-use Git mode did not initialize Git".to_string());
                    }
                    let commit_count = commands
                        .iter()
                        .map(|command| command.match_indices("git commit").count())
                        .sum::<usize>();
                    if commit_count < 2 {
                        errors.push(
                            "First-use Git mode did not create separate baseline and final commits"
                                .to_string(),
                        );
                    }
                    for required in ["git config user.name", "git config user.email"] {
                        if commands.iter().any(|command| command.contains(required)) {
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
        AgentTokenMode::AitLinearSingleSession => {
            for required in ["ait task start", "ait snapshot create", "ait task land"] {
                if commands.iter().any(|command| command.contains(required)) {
                    observed_required_commands.push(required.to_string());
                } else {
                    errors.push(format!(
                        "AIT mode did not execute required command: {required}"
                    ));
                }
            }
            if commands
                .iter()
                .any(|command| command_invokes_git_vcs(command))
            {
                errors.push("AIT mode substituted raw Git workflow commands".to_string());
            }
            for forbidden in [
                "ait-server",
                " --remote",
                "ait push",
                "ait pull",
                "ait remote",
                "ait plan",
                "task start --from",
            ] {
                if commands.iter().any(|command| command.contains(forbidden)) {
                    errors.push(format!(
                        "AIT mode used forbidden solo-local surface: {forbidden}"
                    ));
                }
            }
            match profile {
                AgentTokenAccountingProfile::SteadyStateTaskCost => {
                    for forbidden in ["ait init", "ait config set"] {
                        if commands.iter().any(|command| command.contains(forbidden)) {
                            errors.push(format!(
                                "Steady-state AIT mode repeated first-use bootstrap: {forbidden}"
                            ));
                        }
                    }
                }
                AgentTokenAccountingProfile::FirstUseTotalCost => {
                    for required in ["ait init", "ait config set"] {
                        if commands.iter().any(|command| command.contains(required)) {
                            observed_required_commands.push(required.to_string());
                        } else {
                            errors.push(format!(
                                "First-use AIT mode did not execute bootstrap command: {required}"
                            ));
                        }
                    }
                    let snapshot_count = commands
                        .iter()
                        .map(|command| command.match_indices("ait snapshot create").count())
                        .sum::<usize>();
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
        });
    }
    groups.sort_by(|left, right| {
        (&left.workload_id, &left.mode).cmp(&(&right.workload_id, &right.mode))
    });

    let mut comparisons = Vec::new();
    let mut comparison_bootstrap_samples = Vec::new();
    for workload_id in &manifest.workload_ids {
        let git = groups.iter().find(|group| {
            group.workload_id == *workload_id
                && group.mode == AgentTokenMode::GitLinearSingleSession.as_str()
        });
        let ait = groups.iter().find(|group| {
            group.workload_id == *workload_id
                && group.mode == AgentTokenMode::AitLinearSingleSession.as_str()
        });
        let git_effective = git.and_then(|group| group.effective_tokens_per_accepted_task);
        let ait_effective = ait.and_then(|group| group.effective_tokens_per_accepted_task);
        let token_savings_percent = match (git_effective, ait_effective) {
            (Some(git_value), Some(ait_value)) if git_value > 0.0 => {
                Some(100.0 * (1.0 - ait_value / git_value))
            }
            _ => None,
        };
        let git_acceptance_rate = git.map_or(0.0, |group| group.acceptance_rate);
        let ait_acceptance_rate = ait.map_or(0.0, |group| group.acceptance_rate);
        let git_runs = runs
            .iter()
            .filter(|run| {
                run.workload_id == *workload_id
                    && run.mode == AgentTokenMode::GitLinearSingleSession
                    && run.valid_attempt
                    && run.usage.is_some()
            })
            .collect::<Vec<_>>();
        let ait_runs = runs
            .iter()
            .filter(|run| {
                run.workload_id == *workload_id
                    && run.mode == AgentTokenMode::AitLinearSingleSession
                    && run.valid_attempt
                    && run.usage.is_some()
            })
            .collect::<Vec<_>>();
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
            git_acceptance_rate,
            ait_acceptance_rate,
            acceptance_rate_deficit_percentage_points: 100.0
                * (git_acceptance_rate - ait_acceptance_rate),
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

    let complete_counts = groups.iter().all(|group| {
        group.attempted_count == manifest.attempts_per_cell
            && group.valid_count == manifest.attempts_per_cell
            && group.valid_count >= manifest.campaign_scope.minimum_attempts()
    });
    if !complete_counts {
        blockers.push("Required per-cell attempt counts are incomplete".to_string());
    }
    if comparisons
        .iter()
        .any(|comparison| comparison.token_savings_percent.is_none())
    {
        blockers
            .push("At least one workload lacks an accepted effective-token comparison".to_string());
    }
    if comparisons
        .iter()
        .any(|comparison| comparison.acceptance_rate_deficit_percentage_points > 5.0)
    {
        blockers.push("AIT acceptance-rate deficit exceeds five percentage points".to_string());
    }
    if aggregate_median_token_savings_percent.is_none() {
        blockers.push("Aggregate median savings is unavailable".to_string());
    }
    if aggregate_token_savings_bootstrap_ci95.is_none_or(|interval| interval[0] <= 0.0) {
        blockers
            .push("Aggregate token-savings bootstrap 95% lower bound is not positive".to_string());
    }
    let claim_eligible = manifest.campaign_scope == AgentTokenCampaignScope::Qualification
        && blockers.is_empty()
        && aggregate_median_token_savings_percent.is_some_and(|saving| saving > 0.0)
        && aggregate_token_savings_bootstrap_ci95.is_some_and(|interval| interval[0] > 0.0);
    if manifest.campaign_scope != AgentTokenCampaignScope::Qualification {
        blockers.push(format!(
            "{} evidence is never claim eligible",
            manifest.campaign_scope.as_str()
        ));
    }

    Ok(AgentTokenReport {
        contract: AGENT_TOKEN_REPORT_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        protocol_revision: manifest.protocol_revision.clone(),
        campaign_scope: manifest.campaign_scope.as_str().to_string(),
        accounting_profile: manifest.accounting_profile.as_str().to_string(),
        model: manifest.model.clone(),
        cache_class: manifest.cache_class.clone(),
        network_policy: manifest.network_policy.clone(),
        generated_at: Utc::now().to_rfc3339(),
        scheduled_run_count: schedule.entry_count,
        observed_run_count: runs.len(),
        invalid_run_count,
        groups,
        comparisons,
        aggregate_median_token_savings_percent,
        aggregate_token_savings_bootstrap_ci95,
        claim_eligible,
        blockers,
        limitations: manifest.limitations.clone(),
    })
}

pub fn load_agent_token_report(path: &Path) -> Result<AgentTokenReport, String> {
    read_json(path, "agent-token report")
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
    if baseline.model != candidate.model {
        blockers.push("model pin differs".to_string());
    }
    if baseline.cache_class != candidate.cache_class {
        blockers.push("cache class differs".to_string());
    }
    if baseline.network_policy != candidate.network_policy {
        blockers.push("network policy differs".to_string());
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
        runs.push(read_json(
            &entry.path().join("run-summary.json"),
            "agent-token run summary",
        )?);
    }
    Ok(runs)
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
    output.push_str(&format!("- Scope: `{}`\n", report.campaign_scope));
    output.push_str(&format!("- Accounting: `{}`\n", report.accounting_profile));
    output.push_str(&format!("- Claim eligible: `{}`\n", report.claim_eligible));
    output.push_str(&format!(
        "- Aggregate median savings: `{}`\n",
        report
            .aggregate_median_token_savings_percent
            .map(|value| format!("{value:.2}%"))
            .unwrap_or_else(|| "n/a".to_string())
    ));
    output.push_str(&format!(
        "- Runs: `{}/{}` observed\n\n",
        report.observed_run_count, report.scheduled_run_count
    ));
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
    output.push_str("\n## AIT vs Git\n\n");
    output.push_str("| Workload | Git effective | AIT effective | Savings | Savings CI95 | Paired runs | AIT acceptance deficit |\n");
    output.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for comparison in &report.comparisons {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {:.1} pp |\n",
            comparison.workload_id,
            display_optional(comparison.git_effective_tokens),
            display_optional(comparison.ait_effective_tokens),
            comparison
                .token_savings_percent
                .map(|value| format!("{value:.2}%"))
                .unwrap_or_else(|| "n/a".to_string()),
            comparison
                .token_savings_bootstrap_ci95
                .map(|value| format!("[{:.2}%, {:.2}%]", value[0], value[1]))
                .unwrap_or_else(|| "n/a".to_string()),
            comparison.paired_valid_attempt_count,
            comparison.acceptance_rate_deficit_percentage_points,
        ));
    }
    if !report.blockers.is_empty() {
        output.push_str("\n## Claim Blockers\n\n");
        for blocker in &report.blockers {
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

fn command_invokes_ait(command: &str) -> bool {
    command.contains(" ait ")
        || command.contains("; ait ")
        || command.contains("&& ait ")
        || command.contains("/ait ")
        || command.trim_start().starts_with("ait ")
}

fn command_invokes_git_vcs(command: &str) -> bool {
    [
        "git status",
        "git diff",
        "git log",
        "git show",
        "git branch",
        "git rev-parse",
        "git add",
        "git commit",
        "git checkout",
        "git switch",
    ]
    .iter()
    .any(|needle| command.contains(needle))
}

fn command_family(command: &str) -> &'static str {
    if command_invokes_ait(command) || command.contains("/ait ") {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> AgentTokenModelPin {
        AgentTokenModelPin {
            provider: "openai".to_string(),
            model_id: "gpt-test".to_string(),
            model_revision: "test-revision".to_string(),
            reasoning_effort: "medium".to_string(),
        }
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
            "/bin/zsh -lc 'ait task start --title fix --intent fix'",
            "/bin/zsh -lc 'npm test'",
            "/bin/zsh -lc 'ait snapshot create --message fix'",
            "/bin/zsh -lc 'ait task land LCT-1 --local'",
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
    fn git_transcript_rejects_candidate_metadata_context_override() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("codex.jsonl");
        let events = [
            "/bin/zsh -lc 'git status --short'",
            "/bin/zsh -lc 'npm test'",
            "/bin/zsh -lc 'GIT_DIR=/tmp/copied-git git commit -m repair'",
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
            AgentTokenAccountingProfile::SteadyStateTaskCost,
        )
        .unwrap();
        assert!(!transcript.valid);
        assert!(transcript.errors.iter().any(|error| error
            .contains("overrode the runner-owned isolated repository metadata context")));
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
    fn schedule_is_seeded_block_randomized_and_exact() {
        let manifest = AgentTokenCampaignManifest {
            contract: AGENT_TOKEN_CAMPAIGN_CONTRACT.to_string(),
            campaign_id: "smoke".to_string(),
            protocol_revision: AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
            campaign_scope: AgentTokenCampaignScope::Smoke,
            accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
            seed: 42,
            attempts_per_cell: 2,
            workload_ids: vec!["GD-01".to_string(), "GD-02".to_string()],
            modes: vec![
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ],
            model: model(),
            runtime: AgentTokenRuntime {
                codex_program: PathBuf::from("codex"),
                ait_program: PathBuf::from("ait"),
                git_program: PathBuf::from("git"),
                node_program: PathBuf::from("node"),
                browser_program: None,
                fixture_manifest: PathBuf::from("fixture"),
                run_timeout_seconds: 60,
                ait_first_use_worktree_add_dir: None,
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
    fn report_counts_valid_failures_and_requires_bootstrap_claim_boundary() {
        let manifest = AgentTokenCampaignManifest {
            contract: AGENT_TOKEN_CAMPAIGN_CONTRACT.to_string(),
            campaign_id: "smoke".to_string(),
            protocol_revision: AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
            campaign_scope: AgentTokenCampaignScope::Smoke,
            accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
            seed: 42,
            attempts_per_cell: 3,
            workload_ids: vec!["GD-01".to_string()],
            modes: vec![
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ],
            model: model(),
            runtime: AgentTokenRuntime {
                codex_program: PathBuf::from("codex"),
                ait_program: PathBuf::from("ait"),
                git_program: PathBuf::from("git"),
                node_program: PathBuf::from("node"),
                browser_program: None,
                fixture_manifest: PathBuf::from("fixture"),
                run_timeout_seconds: 60,
                ait_first_use_worktree_add_dir: None,
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
            .map(|entry| match entry.mode {
                AgentTokenMode::GitLinearSingleSession => {
                    synthetic_run(entry, entry.attempt != 3, 100)
                }
                AgentTokenMode::AitLinearSingleSession => synthetic_run(entry, true, 60),
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
        assert!(report.comparisons[0].token_savings_bootstrap_ci95.is_some());
        assert_eq!(report.aggregate_median_token_savings_percent, Some(60.0));
        assert!(
            !report.claim_eligible,
            "smoke evidence never supports a claim"
        );
    }
}
