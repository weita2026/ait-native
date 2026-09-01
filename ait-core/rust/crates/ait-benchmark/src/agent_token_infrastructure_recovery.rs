use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    sha256_digest, AgentTokenCampaignManifest, AgentTokenEnvironment,
    AgentTokenExecutorPreflightEnvironment, AgentTokenMode, AgentTokenRunSummary,
    AgentTokenSchedule, AgentTokenScheduleEntry, AGENT_TOKEN_RECOVERED_SPAWN_CAMPAIGN_ID,
    AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX, AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID,
};

pub const AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_CONTRACT: &str =
    "ait-agent-token-infrastructure-pair-recovery/v1";
pub const AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_SELECTION_FILE: &str =
    "infrastructure-pair-recovery.json";
pub const AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_POLICY_REVISION: &str =
    "game-development-2026-08-28.33";
pub const AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_PAIR_ADMISSION_POLICY: &str =
    "exact_protocol_valid_pair_with_transparent_whole_pair_infrastructure_recovery";
pub const AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_REASON: &str =
    "Repository-owner-authorized transparent whole-pair recovery of a recognized executor infrastructure failure";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentTokenInfrastructureRecoveryArtifact {
    pub run_id: String,
    pub run_summary: String,
    pub run_summary_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenInfrastructurePairRecoverySelection {
    pub contract: String,
    pub campaign_id: String,
    pub source_protocol_revision: String,
    pub policy_revision: String,
    pub source_pair_start_index: usize,
    pub workload_id: String,
    pub attempt: usize,
    pub source_schedule_run_ids: Vec<String>,
    pub observed_source_runs: Vec<AgentTokenInfrastructureRecoveryArtifact>,
    pub replacement_runs: Vec<AgentTokenInfrastructureRecoveryArtifact>,
    pub recovery_runner_sha256: String,
    pub reason: String,
    pub selected_at: String,
}

#[derive(Clone, Debug)]
pub struct AgentTokenInfrastructureRecoveryView {
    pub effective_schedule: AgentTokenSchedule,
    pub effective_runs: Vec<AgentTokenRunSummary>,
    pub excluded_runs: Vec<AgentTokenRunSummary>,
    pub effective_run_summary_paths: BTreeMap<String, PathBuf>,
    pub excluded_run_summary_paths: BTreeMap<String, PathBuf>,
    pub selection: AgentTokenInfrastructurePairRecoverySelection,
}

pub fn recognized_infrastructure_failure(value: &str) -> bool {
    matches!(
        value,
        "codex_tool_process_spawn_failure"
            | "provider_usage_limit"
            | "provider_rate_limit"
            | "provider_authentication_failure"
            | "provider_model_unavailable"
            | "provider_transport_failure"
            | "provider_runtime_error_event"
            | "provider_session_failed_before_candidate_execution"
    )
}

pub fn replacement_run_id(source_run_id: &str) -> String {
    format!("{source_run_id}-infra-recovery-01")
}

pub fn load_agent_token_infrastructure_recovery_view(
    manifest: &AgentTokenCampaignManifest,
    source_schedule: &AgentTokenSchedule,
    campaign_dir: &Path,
    source_runs: &[AgentTokenRunSummary],
) -> Result<Option<AgentTokenInfrastructureRecoveryView>, String> {
    load_agent_token_infrastructure_recovery_view_with_additional_gap(
        manifest,
        source_schedule,
        campaign_dir,
        source_runs,
        None,
    )
}

pub(crate) fn load_agent_token_infrastructure_recovery_view_with_additional_gap(
    manifest: &AgentTokenCampaignManifest,
    source_schedule: &AgentTokenSchedule,
    campaign_dir: &Path,
    source_runs: &[AgentTokenRunSummary],
    additional_gap_pair_start: Option<usize>,
) -> Result<Option<AgentTokenInfrastructureRecoveryView>, String> {
    let selection_path = campaign_dir.join(AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_SELECTION_FILE);
    if !selection_path.exists() {
        return Ok(None);
    }
    require_regular_file(&selection_path, "infrastructure recovery selection")?;
    let selection = read_json::<AgentTokenInfrastructurePairRecoverySelection>(
        &selection_path,
        "infrastructure recovery selection",
    )?;
    validate_selection_identity(&selection, manifest, source_schedule)?;

    let pair_start = selection.source_pair_start_index;
    let pair_end = pair_start + 2;
    let source_pair = &source_schedule.entries[pair_start..pair_end];
    let mut by_id = source_runs
        .iter()
        .cloned()
        .map(|run| (run.run_id.clone(), run))
        .collect::<BTreeMap<_, _>>();
    if by_id.len() != source_runs.len() {
        return Err("Infrastructure recovery source contains duplicate run IDs".to_string());
    }
    let schedule_ids = source_schedule
        .entries
        .iter()
        .map(|entry| entry.run_id.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(unexpected) = by_id
        .keys()
        .find(|run_id| !schedule_ids.contains(run_id.as_str()))
    {
        return Err(format!(
            "Infrastructure recovery source run {unexpected} is absent from the frozen schedule"
        ));
    }

    let mut effective_runs = Vec::with_capacity(source_schedule.entries.len());
    let mut effective_paths = BTreeMap::new();
    for entry in &source_schedule.entries[..pair_start] {
        let run = by_id.remove(&entry.run_id).ok_or_else(|| {
            format!(
                "Infrastructure recovery source prefix is missing {}",
                entry.run_id
            )
        })?;
        require_run_identity(&run, entry, manifest)?;
        if !run.valid_attempt {
            return Err(format!(
                "Infrastructure recovery source prefix run {} is invalid before the contaminated pair",
                run.run_id
            ));
        }
        effective_paths.insert(
            run.run_id.clone(),
            raw_summary_path(campaign_dir, &run.run_id),
        );
        effective_runs.push(run);
    }

    let mut excluded_runs = Vec::new();
    let mut excluded_paths = BTreeMap::new();
    for entry in source_pair {
        if let Some(run) = by_id.remove(&entry.run_id) {
            require_run_identity(&run, entry, manifest)?;
            excluded_paths.insert(
                run.run_id.clone(),
                raw_summary_path(campaign_dir, &run.run_id),
            );
            excluded_runs.push(run);
        }
    }
    if excluded_runs.is_empty()
        || !excluded_runs.iter().any(|run| {
            run.infrastructure_failure
                .as_deref()
                .is_some_and(recognized_infrastructure_failure)
        })
    {
        return Err(
            "Infrastructure recovery source pair lacks a recognized infrastructure failure"
                .to_string(),
        );
    }
    validate_observed_source_artifacts(campaign_dir, &excluded_runs, &selection)?;

    let replacement_entries = source_pair
        .iter()
        .map(|entry| {
            let mut replacement = entry.clone();
            replacement.run_id = replacement_run_id(&entry.run_id);
            replacement
        })
        .collect::<Vec<_>>();
    let mut replacement_runs = Vec::new();
    for (entry, artifact) in replacement_entries
        .iter()
        .zip(selection.replacement_runs.iter())
    {
        if artifact.run_id != entry.run_id {
            return Err(format!(
                "Infrastructure replacement artifact {} differs from expected {}",
                artifact.run_id, entry.run_id
            ));
        }
        let path = campaign_dir.join(portable_relative_path(&artifact.run_summary)?);
        require_regular_file(&path, "infrastructure replacement run summary")?;
        require_digest(
            &path,
            &artifact.run_summary_sha256,
            "infrastructure replacement run summary",
        )?;
        let run =
            read_json::<AgentTokenRunSummary>(&path, "infrastructure replacement run summary")?;
        require_run_identity(&run, entry, manifest)?;
        require_admitted_replacement(&run)?;
        validate_replacement_run_files(manifest, campaign_dir, &run, &path)?;
        effective_paths.insert(run.run_id.clone(), path);
        replacement_runs.push(run);
    }
    if replacement_runs.len() != 2 {
        return Err("Infrastructure recovery requires exactly two replacement lanes".to_string());
    }
    effective_runs.extend(replacement_runs);

    let mut encountered_gap = false;
    let mut suffix_count = 0_usize;
    for (index, entry) in source_schedule.entries[pair_end..].iter().enumerate() {
        let schedule_index = pair_end + index;
        if additional_gap_pair_start
            .is_some_and(|start| (start..start.saturating_add(2)).contains(&schedule_index))
        {
            if by_id.contains_key(&entry.run_id) {
                return Err(format!(
                    "Infrastructure recovery additional gap unexpectedly contains completed run {}",
                    entry.run_id
                ));
            }
            continue;
        }
        match by_id.remove(&entry.run_id) {
            Some(_) if encountered_gap => {
                return Err(format!(
                    "Infrastructure recovery source run {} occurs after a missing suffix entry",
                    entry.run_id
                ));
            }
            Some(run) => {
                require_run_identity(&run, entry, manifest)?;
                if !run.valid_attempt {
                    return Err(format!(
                        "Infrastructure recovery suffix run {} is invalid; a second contaminated pair is not admitted",
                        run.run_id
                    ));
                }
                effective_paths.insert(
                    run.run_id.clone(),
                    raw_summary_path(campaign_dir, &run.run_id),
                );
                effective_runs.push(run);
                suffix_count += 1;
            }
            None => encountered_gap = true,
        }
    }
    if !by_id.is_empty() {
        return Err("Infrastructure recovery source contains unexpected residual runs".to_string());
    }
    let skipped_gap_lanes = usize::from(
        additional_gap_pair_start
            .is_some_and(|start| start < AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX),
    ) * 2;
    let recovered_spawn_partial = suffix_count % 2 == 1
        && manifest.campaign_id == AGENT_TOKEN_RECOVERED_SPAWN_CAMPAIGN_ID
        && effective_runs.len()
            == AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX + 1 - skipped_gap_lanes
        && effective_runs
            .last()
            .is_some_and(|run| run.run_id == AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID)
        && campaign_dir
            .join("adjudications")
            .join(format!("{AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID}.json"))
            .is_file()
        && source_schedule
            .entries
            .get(AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX)
            .is_some_and(|entry| entry.run_id == AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID)
        && source_schedule
            .entries
            .get(AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX + 1)
            .is_some_and(|entry| !campaign_dir.join("runs").join(&entry.run_id).exists());
    if !suffix_count.is_multiple_of(2) && !recovered_spawn_partial {
        return Err(format!(
            "Infrastructure recovery suffix ends with a partial pair after {suffix_count} runs"
        ));
    }

    let runner = campaign_dir.join("infrastructure-recoveries/recovery-0001/recovery-runner");
    require_regular_file(&runner, "infrastructure recovery runner")?;
    require_digest(
        &runner,
        &selection.recovery_runner_sha256,
        "infrastructure recovery runner",
    )?;

    let mut effective_schedule = source_schedule.clone();
    for (entry, replacement) in effective_schedule.entries[pair_start..pair_end]
        .iter_mut()
        .zip(replacement_entries)
    {
        entry.run_id = replacement.run_id;
    }
    Ok(Some(AgentTokenInfrastructureRecoveryView {
        effective_schedule,
        effective_runs,
        excluded_runs,
        effective_run_summary_paths: effective_paths,
        excluded_run_summary_paths: excluded_paths,
        selection,
    }))
}

pub(crate) fn validate_selection_identity(
    selection: &AgentTokenInfrastructurePairRecoverySelection,
    manifest: &AgentTokenCampaignManifest,
    schedule: &AgentTokenSchedule,
) -> Result<(), String> {
    if selection.contract != AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_CONTRACT
        || selection.campaign_id != manifest.campaign_id
        || selection.source_protocol_revision != manifest.protocol_revision
        || selection.policy_revision != AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_POLICY_REVISION
        || selection.reason != AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_REASON
        || selection.selected_at.trim().is_empty()
    {
        return Err("Infrastructure recovery selection identity differs".to_string());
    }
    if !selection.source_pair_start_index.is_multiple_of(2)
        || selection.source_pair_start_index + 2 > schedule.entries.len()
    {
        return Err(
            "Infrastructure recovery pair index is outside the frozen schedule".to_string(),
        );
    }
    let pair =
        &schedule.entries[selection.source_pair_start_index..selection.source_pair_start_index + 2];
    if pair[0].workload_id != selection.workload_id
        || pair[0].attempt != selection.attempt
        || pair.iter().any(|entry| {
            entry.workload_id != selection.workload_id || entry.attempt != selection.attempt
        })
        || pair.iter().map(|entry| entry.mode).collect::<BTreeSet<_>>()
            != BTreeSet::from([
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ])
        || selection.source_schedule_run_ids
            != pair
                .iter()
                .map(|entry| entry.run_id.clone())
                .collect::<Vec<_>>()
        || selection.replacement_runs.len() != 2
        || selection.observed_source_runs.is_empty()
        || selection.observed_source_runs.len() > 2
    {
        return Err(
            "Infrastructure recovery pair identity differs from the frozen schedule".to_string(),
        );
    }
    validate_digest("recovery runner", &selection.recovery_runner_sha256)?;
    Ok(())
}

fn validate_observed_source_artifacts(
    campaign_dir: &Path,
    runs: &[AgentTokenRunSummary],
    selection: &AgentTokenInfrastructurePairRecoverySelection,
) -> Result<(), String> {
    let expected = runs
        .iter()
        .map(|run| {
            let relative = format!("runs/{}/run-summary.json", run.run_id);
            let path = campaign_dir.join(&relative);
            let bytes = fs::read(&path).map_err(|error| {
                format!(
                    "Failed to read infrastructure source summary {}: {error}",
                    path.display()
                )
            })?;
            Ok(AgentTokenInfrastructureRecoveryArtifact {
                run_id: run.run_id.clone(),
                run_summary: relative,
                run_summary_sha256: sha256_digest(&bytes),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if selection.observed_source_runs != expected {
        return Err(
            "Infrastructure recovery source artifacts differ from immutable evidence".to_string(),
        );
    }
    Ok(())
}

pub(crate) fn require_run_identity(
    run: &AgentTokenRunSummary,
    entry: &AgentTokenScheduleEntry,
    manifest: &AgentTokenCampaignManifest,
) -> Result<(), String> {
    if run.campaign_id != manifest.campaign_id
        || run.run_id != entry.run_id
        || run.workload_id != entry.workload_id
        || run.mode != entry.mode
        || run.accounting_profile != manifest.accounting_profile
        || run.attempt != entry.attempt
        || run.block_index != entry.block_index
        || run.randomized_order != entry.randomized_order
    {
        return Err(format!(
            "Infrastructure recovery run {} identity differs from its schedule entry",
            run.run_id
        ));
    }
    Ok(())
}

fn require_admitted_replacement(run: &AgentTokenRunSummary) -> Result<(), String> {
    if !run.valid_attempt
        || !run.accepted_equivalent
        || !run.evaluator_accepted
        || run.browser.status != "passed"
        || !run.workflow_closed
        || run.infrastructure_failure.is_some()
        || !run.invalid_reasons.is_empty()
        || !run.failure_reasons.is_empty()
        || run.usage.is_none()
        || !run.transcript.valid
        || !run.transcript.errors.is_empty()
    {
        return Err(format!(
            "Infrastructure replacement run {} did not pass every admission gate",
            run.run_id
        ));
    }
    let usage = run.usage.as_ref().expect("replacement usage was checked");
    if usage.run_id != run.run_id
        || usage.workload_id != run.workload_id
        || usage.mode != run.mode
        || usage.accounting_profile != run.accounting_profile
    {
        return Err(format!(
            "Infrastructure replacement run {} usage linkage differs",
            run.run_id
        ));
    }
    Ok(())
}

pub(crate) fn validate_replacement_run_files(
    manifest: &AgentTokenCampaignManifest,
    campaign_dir: &Path,
    run: &AgentTokenRunSummary,
    summary_path: &Path,
) -> Result<(), String> {
    let run_dir = summary_path.parent().ok_or_else(|| {
        format!(
            "Replacement summary has no parent: {}",
            summary_path.display()
        )
    })?;
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
        require_regular_file(
            &run_dir.join(required),
            &format!("infrastructure replacement {required}"),
        )?;
    }
    for identical in ["campaign-manifest.json", "fixture-manifest.json"] {
        if fs::read(campaign_dir.join(identical)).map_err(|error| {
            format!("Failed to read source recovery evidence {identical}: {error}")
        })? != fs::read(run_dir.join(identical)).map_err(|error| {
            format!("Failed to read replacement recovery evidence {identical}: {error}")
        })? {
            return Err(format!(
                "Infrastructure replacement {identical} differs from the source campaign"
            ));
        }
    }
    let environment = read_json::<AgentTokenEnvironment>(
        &run_dir.join("environment.json"),
        "infrastructure replacement environment",
    )?;
    let source_environment = read_json::<AgentTokenExecutorPreflightEnvironment>(
        &campaign_dir.join("executor-preflight-environment.json"),
        "source executor preflight environment",
    )?;
    if environment.network_policy != manifest.network_policy
        || environment.cache_class != manifest.cache_class
        || environment.project_doc_max_bytes != manifest.runtime.project_doc_max_bytes
        || environment.codex_version != source_environment.codex_version
    {
        return Err(format!(
            "Infrastructure replacement run {} environment differs",
            run.run_id
        ));
    }
    Ok(())
}

fn raw_summary_path(campaign_dir: &Path, run_id: &str) -> PathBuf {
    campaign_dir
        .join("runs")
        .join(run_id)
        .join("run-summary.json")
}

fn portable_relative_path(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "Infrastructure recovery path must be portable and relative: {value:?}"
        ));
    }
    Ok(path.to_path_buf())
}

fn validate_digest(label: &str, digest: &str) -> Result<(), String> {
    let value = digest.strip_prefix("sha256:").unwrap_or(digest);
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "Infrastructure recovery {label} SHA-256 is malformed"
        ));
    }
    Ok(())
}

fn require_digest(path: &Path, expected: &str, label: &str) -> Result<(), String> {
    validate_digest(label, expected)?;
    let bytes = fs::read(path)
        .map_err(|error| format!("Failed to read {label} {}: {error}", path.display()))?;
    let actual = sha256_digest(&bytes);
    if actual != expected && format!("sha256:{actual}") != expected {
        return Err(format!(
            "Infrastructure recovery {label} digest differs: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

fn require_regular_file(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Failed to inspect {label} {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "{label} must be a regular file: {}",
            path.display()
        ));
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Failed to read {label} {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Failed to decode {label} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_ids_are_distinct_and_only_executor_failures_are_recognized() {
        assert_eq!(
            replacement_run_id("campaign-b002-gd-02-git"),
            "campaign-b002-gd-02-git-infra-recovery-01"
        );
        assert!(recognized_infrastructure_failure(
            "codex_tool_process_spawn_failure"
        ));
        assert!(recognized_infrastructure_failure(
            "provider_transport_failure"
        ));
        assert!(!recognized_infrastructure_failure(
            "evaluator_rejected_candidate"
        ));
        assert!(!recognized_infrastructure_failure("high_token_usage"));
    }
}
