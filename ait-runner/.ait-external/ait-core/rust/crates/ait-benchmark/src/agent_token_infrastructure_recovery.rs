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
    /// Which numbered recovery directory holds this pair's replacement lanes.
    /// A legacy single-object selection predates numbering and is always the
    /// first recovery.
    #[serde(default = "first_recovery_ordinal")]
    pub recovery_ordinal: usize,
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
    /// Every recovery applied to this campaign, ordered by schedule position.
    pub selections: Vec<AgentTokenInfrastructurePairRecoverySelection>,
}

pub(crate) fn first_recovery_ordinal() -> usize {
    1
}

/// The selection evidence holds an ordered list of pair recoveries. A campaign
/// recorded before numbering carries a single object, which loads as a
/// one-element list.
#[derive(Deserialize)]
#[serde(untagged)]
enum AgentTokenInfrastructureRecoverySelectionFile {
    Many(Vec<AgentTokenInfrastructurePairRecoverySelection>),
    One(Box<AgentTokenInfrastructurePairRecoverySelection>),
}

/// Reads every recorded pair recovery, ordered by schedule position.
pub(crate) fn load_infrastructure_recovery_selections(
    campaign_dir: &Path,
) -> Result<Vec<AgentTokenInfrastructurePairRecoverySelection>, String> {
    let selection_path = campaign_dir.join(AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_SELECTION_FILE);
    if !selection_path.exists() {
        return Ok(Vec::new());
    }
    require_regular_file(&selection_path, "infrastructure recovery selection")?;
    let file = read_json::<AgentTokenInfrastructureRecoverySelectionFile>(
        &selection_path,
        "infrastructure recovery selection",
    )?;
    let mut selections = match file {
        AgentTokenInfrastructureRecoverySelectionFile::Many(many) => many,
        AgentTokenInfrastructureRecoverySelectionFile::One(one) => vec![*one],
    };
    selections.sort_by_key(|selection| selection.source_pair_start_index);
    Ok(selections)
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

pub fn replacement_run_id(source_run_id: &str, recovery_ordinal: usize) -> String {
    format!("{source_run_id}-infra-recovery-{recovery_ordinal:02}")
}

/// The numbered directory holding one recovery's replacement lanes.
pub fn recovery_directory(recovery_ordinal: usize) -> String {
    format!("infrastructure-recoveries/recovery-{recovery_ordinal:04}")
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
    let selections = load_infrastructure_recovery_selections(campaign_dir)?;
    if selections.is_empty() {
        return Ok(None);
    }
    for selection in &selections {
        validate_selection_identity(selection, manifest, source_schedule)?;
    }
    // Recoveries are ordered by schedule position and each pair is recovered at
    // most once, so applying them in order rebuilds an exact schedule prefix.
    let mut previous_pair_start: Option<usize> = None;
    for selection in &selections {
        if previous_pair_start.is_some_and(|previous| selection.source_pair_start_index <= previous)
        {
            return Err(
                "Infrastructure recoveries are not ordered by distinct schedule pairs".to_string(),
            );
        }
        previous_pair_start = Some(selection.source_pair_start_index);
    }
    let ordinals = selections
        .iter()
        .map(|selection| selection.recovery_ordinal)
        .collect::<BTreeSet<_>>();
    if ordinals.len() != selections.len() || ordinals.contains(&0) {
        return Err("Infrastructure recovery ordinals are not distinct".to_string());
    }

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
    let recovered_by_pair_start = selections
        .iter()
        .map(|selection| (selection.source_pair_start_index, selection))
        .collect::<BTreeMap<_, _>>();

    let mut effective_runs = Vec::with_capacity(source_schedule.entries.len());
    let mut effective_paths = BTreeMap::new();
    let mut excluded_runs = Vec::new();
    let mut excluded_paths = BTreeMap::new();
    let mut effective_schedule = source_schedule.clone();
    let mut encountered_gap = false;
    // Counted from the last recovered pair so the trailing-parity guard keeps
    // its original meaning.
    let mut runs_after_last_recovery = 0_usize;

    for pair_start in (0..source_schedule.entries.len()).step_by(2) {
        let pair_end = pair_start + 2;
        if additional_gap_pair_start.is_some_and(|start| start == pair_start) {
            for entry in &source_schedule.entries[pair_start..pair_end] {
                if by_id.contains_key(&entry.run_id) {
                    return Err(format!(
                        "Infrastructure recovery additional gap unexpectedly contains completed run {}",
                        entry.run_id
                    ));
                }
            }
            continue;
        }
        let Some(selection) = recovered_by_pair_start.get(&pair_start) else {
            for entry in &source_schedule.entries[pair_start..pair_end] {
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
                                "Infrastructure recovery run {} is invalid and no recorded recovery covers its pair",
                                run.run_id
                            ));
                        }
                        effective_paths.insert(
                            run.run_id.clone(),
                            raw_summary_path(campaign_dir, &run.run_id),
                        );
                        effective_runs.push(run);
                        runs_after_last_recovery += 1;
                    }
                    None => encountered_gap = true,
                }
            }
            continue;
        };

        let source_pair = &source_schedule.entries[pair_start..pair_end];
        let mut pair_excluded = Vec::new();
        for entry in source_pair {
            if let Some(run) = by_id.remove(&entry.run_id) {
                require_run_identity(&run, entry, manifest)?;
                excluded_paths.insert(
                    run.run_id.clone(),
                    raw_summary_path(campaign_dir, &run.run_id),
                );
                pair_excluded.push(run);
            }
        }
        if pair_excluded.is_empty()
            || !pair_excluded.iter().any(|run| {
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
        validate_observed_source_artifacts(campaign_dir, &pair_excluded, selection)?;
        excluded_runs.extend(pair_excluded);

        let replacement_entries = source_pair
            .iter()
            .map(|entry| {
                let mut replacement = entry.clone();
                replacement.run_id = replacement_run_id(&entry.run_id, selection.recovery_ordinal);
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
            return Err(
                "Infrastructure recovery requires exactly two replacement lanes".to_string(),
            );
        }
        effective_runs.extend(replacement_runs);
        for (entry, replacement) in effective_schedule.entries[pair_start..pair_end]
            .iter_mut()
            .zip(replacement_entries)
        {
            entry.run_id = replacement.run_id;
        }

        let runner = campaign_dir
            .join(recovery_directory(selection.recovery_ordinal))
            .join("recovery-runner");
        require_regular_file(&runner, "infrastructure recovery runner")?;
        require_digest(
            &runner,
            &selection.recovery_runner_sha256,
            "infrastructure recovery runner",
        )?;
        runs_after_last_recovery = 0;
    }
    if !by_id.is_empty() {
        return Err("Infrastructure recovery source contains unexpected residual runs".to_string());
    }

    let skipped_gap_lanes = usize::from(
        additional_gap_pair_start
            .is_some_and(|start| start < AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX),
    ) * 2;
    let recovered_spawn_partial = runs_after_last_recovery % 2 == 1
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
    if !runs_after_last_recovery.is_multiple_of(2) && !recovered_spawn_partial {
        return Err(format!(
            "Infrastructure recovery suffix ends with a partial pair after {runs_after_last_recovery} runs"
        ));
    }

    Ok(Some(AgentTokenInfrastructureRecoveryView {
        effective_schedule,
        effective_runs,
        excluded_runs,
        effective_run_summary_paths: effective_paths,
        excluded_run_summary_paths: excluded_paths,
        selections,
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

/// The sole failure reason that restates the functional outcome rather than
/// reporting anything about measurement validity.
pub(crate) const FUNCTIONAL_ACCEPTANCE_REJECTED_REASON: &str =
    "functional acceptance rejected the candidate";

/// Infrastructure recovery replaces a run lost to infrastructure, so admission
/// asks whether the replacement is a valid, uncontaminated measurement — not
/// whether the model scored well on it. Gating on the functional outcome re-runs
/// a lost pair until the model happens to clear the threshold, so the campaign
/// keeps the model's good runs and discards its ordinary ones; that is
/// outcome-conditioned selection on exactly the quantity being reported. `.52`
/// removed the conflation at the pair level and this removes it at the run
/// level. A campaign declaring `functional_replacement_policy: "none"` has
/// already stated that a low-scoring valid run stays in the data as data.
fn require_admitted_replacement(run: &AgentTokenRunSummary) -> Result<(), String> {
    let blocking_failure_reasons = run
        .failure_reasons
        .iter()
        .any(|reason| reason != FUNCTIONAL_ACCEPTANCE_REJECTED_REASON);
    if !run.valid_attempt
        || run.browser.status != "passed"
        || !run.workflow_closed
        || run.infrastructure_failure.is_some()
        || !run.invalid_reasons.is_empty()
        || blocking_failure_reasons
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

    /// A real replacement run that was blocked by the previous gate: the
    /// evaluator scored it 85 against a 90 threshold while every
    /// measurement-validity condition passed.
    const VALID_LOW_SCORE_REPLACEMENT: &str = include_str!(
        "../fixtures/agent-token-recovery/valid-low-score-replacement-run-summary.json"
    );

    fn valid_low_score_replacement() -> AgentTokenRunSummary {
        serde_json::from_str(VALID_LOW_SCORE_REPLACEMENT)
            .expect("recorded replacement run summary decodes")
    }

    #[test]
    fn replacement_admission_reads_measurement_validity_not_functional_outcome() {
        let run = valid_low_score_replacement();
        // The fixture is the case the old gate rejected.
        assert!(!run.evaluator_accepted);
        assert!(!run.accepted_equivalent);
        assert_eq!(
            run.failure_reasons,
            vec![FUNCTIONAL_ACCEPTANCE_REJECTED_REASON.to_string()]
        );
        assert!(run.valid_attempt);
        assert!(require_admitted_replacement(&run).is_ok());
    }

    #[test]
    fn replacement_admission_still_rejects_an_invalid_measurement() {
        for (label, break_it) in [
            (
                "invalid attempt",
                Box::new(|run: &mut AgentTokenRunSummary| run.valid_attempt = false)
                    as Box<dyn Fn(&mut AgentTokenRunSummary)>,
            ),
            (
                "infrastructure failure",
                Box::new(|run: &mut AgentTokenRunSummary| {
                    run.infrastructure_failure = Some("provider_usage_limit".to_string());
                }),
            ),
            (
                "invalid reason",
                Box::new(|run: &mut AgentTokenRunSummary| {
                    run.invalid_reasons = vec!["usage stream truncated".to_string()];
                }),
            ),
            (
                "non-functional failure reason",
                Box::new(|run: &mut AgentTokenRunSummary| {
                    run.failure_reasons = vec!["workflow verification failed".to_string()];
                }),
            ),
            (
                "browser not passed",
                Box::new(|run: &mut AgentTokenRunSummary| {
                    run.browser.status = "failed".to_string();
                }),
            ),
            (
                "workflow not closed",
                Box::new(|run: &mut AgentTokenRunSummary| run.workflow_closed = false),
            ),
            (
                "usage missing",
                Box::new(|run: &mut AgentTokenRunSummary| run.usage = None),
            ),
            (
                "transcript invalid",
                Box::new(|run: &mut AgentTokenRunSummary| run.transcript.valid = false),
            ),
        ] {
            let mut run = valid_low_score_replacement();
            break_it(&mut run);
            assert!(
                require_admitted_replacement(&run).is_err(),
                "{label} must still be rejected"
            );
        }
    }

    #[test]
    fn recovery_ids_are_distinct_and_only_executor_failures_are_recognized() {
        assert_eq!(
            replacement_run_id("campaign-b002-gd-02-git", 1),
            "campaign-b002-gd-02-git-infra-recovery-01"
        );
        // A later interruption in the same campaign gets its own ordinal and
        // its own numbered directory.
        assert_eq!(
            replacement_run_id("campaign-b004-gd-04-ait", 2),
            "campaign-b004-gd-04-ait-infra-recovery-02"
        );
        assert_eq!(
            recovery_directory(1),
            "infrastructure-recoveries/recovery-0001"
        );
        assert_eq!(
            recovery_directory(2),
            "infrastructure-recoveries/recovery-0002"
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
