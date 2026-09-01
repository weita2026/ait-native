use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::agent_token_infrastructure_recovery::{
    load_agent_token_infrastructure_recovery_view_with_additional_gap, require_run_identity,
    validate_replacement_run_files,
};
use crate::{
    sha256_digest, AgentTokenCampaignManifest, AgentTokenInfrastructurePairRecoverySelection,
    AgentTokenInfrastructureRecoveryArtifact, AgentTokenMode, AgentTokenRunSummary,
    AgentTokenSchedule, AgentTokenScheduleEntry,
};

pub const AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_CONTRACT: &str =
    "ait-agent-token-host-shutdown-pair-recovery/v1";
pub const AGENT_TOKEN_HOST_SHUTDOWN_OBSERVATION_CONTRACT: &str =
    "ait-agent-token-host-shutdown-observation/v1";
pub const AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_SELECTION_FILE: &str =
    "host-shutdown-pair-recovery.json";
pub const AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_POLICY_REVISION: &str =
    "game-development-2026-08-29.34";
pub const AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_PAIR_ADMISSION_POLICY: &str =
    "exact_protocol_valid_pairs_with_transparent_infrastructure_and_host_shutdown_whole_pair_recovery";
pub const AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_REASON: &str =
    "Repository-owner-authorized transparent whole-pair recovery of the evidenced 2026-08-29 host shutdown";

pub const AGENT_TOKEN_HOST_SHUTDOWN_CAMPAIGN_ID: &str =
    "game-v1-g56s-max-sprint-on-natural-complete200-20260828";
pub const AGENT_TOKEN_HOST_SHUTDOWN_INTERRUPTED_RUN_ID: &str =
    "game-v1-g56s-max-sprint-on-natural-complete200-20260828-b009-gd-05-git";
pub const AGENT_TOKEN_HOST_SHUTDOWN_PAIR_START_INDEX: usize = 80;
pub const AGENT_TOKEN_HOST_SHUTDOWN_AT_UNIX_S: u64 = 1_787_940_840;
pub const AGENT_TOKEN_HOST_REBOOT_AT_UNIX_S: u64 = 1_787_941_172;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentTokenInterruptedArtifact {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenHostShutdownObservation {
    pub contract: String,
    pub captured_at: String,
    pub shutdown_at_unix_s: u64,
    pub reboot_at_unix_s: u64,
    pub interrupted_event_mtime_unix_s: u64,
    pub last_command: String,
    pub last_output: String,
    pub kern_boottime_command: String,
    pub kern_boottime_output: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenHostShutdownPairRecoverySelection {
    pub contract: String,
    pub campaign_id: String,
    pub source_protocol_revision: String,
    pub policy_revision: String,
    pub source_pair_start_index: usize,
    pub workload_id: String,
    pub attempt: usize,
    pub source_schedule_run_ids: Vec<String>,
    pub interrupted_run_id: String,
    pub interrupted_run_directory: String,
    pub interrupted_artifacts: Vec<AgentTokenInterruptedArtifact>,
    pub interrupted_event_path: String,
    pub interrupted_event_sha256: String,
    pub interrupted_event_mtime_unix_s: u64,
    pub terminal_provider_event_observed: bool,
    pub run_summary_observed: bool,
    pub host_observation: String,
    pub host_observation_sha256: String,
    pub replacement_runs: Vec<AgentTokenInfrastructureRecoveryArtifact>,
    pub recovery_runner_sha256: String,
    pub reason: String,
    pub selected_at: String,
}

#[derive(Clone, Debug)]
pub struct AgentTokenHostShutdownInterruption {
    pub run_directory: PathBuf,
    pub artifacts: Vec<AgentTokenInterruptedArtifact>,
    pub event_path: PathBuf,
    pub event_sha256: String,
    pub event_mtime_unix_s: u64,
}

#[derive(Clone, Debug)]
pub struct AgentTokenHostShutdownRecoveryView {
    pub effective_schedule: AgentTokenSchedule,
    pub effective_runs: Vec<AgentTokenRunSummary>,
    pub excluded_runs: Vec<AgentTokenRunSummary>,
    pub effective_run_summary_paths: BTreeMap<String, PathBuf>,
    pub excluded_run_summary_paths: BTreeMap<String, PathBuf>,
    pub infrastructure_selection: AgentTokenInfrastructurePairRecoverySelection,
    pub selection: AgentTokenHostShutdownPairRecoverySelection,
}

pub fn host_shutdown_replacement_run_id(source_run_id: &str) -> String {
    format!("{source_run_id}-host-shutdown-recovery-01")
}

pub fn classify_host_shutdown_interruption(
    manifest: &AgentTokenCampaignManifest,
    schedule: &AgentTokenSchedule,
    campaign_dir: &Path,
    effective_prefix_len: usize,
) -> Result<AgentTokenHostShutdownInterruption, String> {
    if manifest.campaign_id != AGENT_TOKEN_HOST_SHUTDOWN_CAMPAIGN_ID
        || effective_prefix_len != AGENT_TOKEN_HOST_SHUTDOWN_PAIR_START_INDEX
    {
        return Err("Host-shutdown recovery is not authorized for this campaign or prefix".into());
    }
    let pair = exact_authorized_pair(schedule)?;
    let interrupted = &pair[0];
    if interrupted.run_id != AGENT_TOKEN_HOST_SHUTDOWN_INTERRUPTED_RUN_ID
        || interrupted.mode != AgentTokenMode::GitLinearSingleSession
    {
        return Err("Host-shutdown interrupted lane differs from the exact authorization".into());
    }
    let run_directory = campaign_dir.join("runs").join(&interrupted.run_id);
    require_directory(&run_directory, "host-shutdown interrupted run")?;
    if run_directory.join("run-summary.json").exists() {
        return Err("Host-shutdown interrupted lane unexpectedly has run-summary.json".into());
    }
    let counterpart = campaign_dir.join("runs").join(&pair[1].run_id);
    if counterpart.exists() {
        return Err("Host-shutdown pair counterpart was already materialized".into());
    }
    for required in [
        "campaign-manifest.json",
        "fixture-manifest.json",
        "prompt.txt",
        "run-manifest.json",
        "private/codex-events.raw.jsonl",
    ] {
        require_regular_file(
            &run_directory.join(required),
            &format!("host-shutdown interrupted {required}"),
        )?;
    }
    for absent in [
        "provider-usage.jsonl",
        "acceptance-report.json",
        "browser-report.json",
        "workflow-verification.json",
        "run-summary.json",
    ] {
        if run_directory.join(absent).exists() {
            return Err(format!(
                "Host-shutdown interrupted lane unexpectedly contains {absent}"
            ));
        }
    }
    let event_path = run_directory.join("private/codex-events.raw.jsonl");
    let event_bytes = fs::read(&event_path).map_err(|error| {
        format!(
            "Failed to read host-shutdown event evidence {}: {error}",
            event_path.display()
        )
    })?;
    if event_bytes.is_empty() {
        return Err("Host-shutdown event evidence is empty".into());
    }
    if terminal_provider_event_observed(&event_bytes)? {
        return Err("Host-shutdown lane contains a terminal provider event".into());
    }
    let event_mtime_unix_s = regular_file_mtime_s(&event_path)?;
    if !(AGENT_TOKEN_HOST_SHUTDOWN_AT_UNIX_S
        ..AGENT_TOKEN_HOST_SHUTDOWN_AT_UNIX_S.saturating_add(60))
        .contains(&event_mtime_unix_s)
    {
        return Err(format!(
            "Host-shutdown event mtime {event_mtime_unix_s} is outside the evidenced shutdown minute"
        ));
    }
    let artifacts = inventory_regular_files(&run_directory)?;
    if artifacts.is_empty() {
        return Err("Host-shutdown interrupted inventory is empty".into());
    }
    Ok(AgentTokenHostShutdownInterruption {
        run_directory,
        artifacts,
        event_path,
        event_sha256: sha256_digest(&event_bytes),
        event_mtime_unix_s,
    })
}

pub fn capture_host_shutdown_observation(
    event_mtime_unix_s: u64,
) -> Result<AgentTokenHostShutdownObservation, String> {
    let last_command = "/usr/bin/last reboot shutdown";
    let last_output = command_output("/usr/bin/last", &["reboot", "shutdown"])?;
    let kern_boottime_command = "/usr/sbin/sysctl -n kern.boottime";
    let kern_boottime_output = command_output("/usr/sbin/sysctl", &["-n", "kern.boottime"])?;
    let observation = AgentTokenHostShutdownObservation {
        contract: AGENT_TOKEN_HOST_SHUTDOWN_OBSERVATION_CONTRACT.to_string(),
        captured_at: Utc::now().to_rfc3339(),
        shutdown_at_unix_s: AGENT_TOKEN_HOST_SHUTDOWN_AT_UNIX_S,
        reboot_at_unix_s: AGENT_TOKEN_HOST_REBOOT_AT_UNIX_S,
        interrupted_event_mtime_unix_s: event_mtime_unix_s,
        last_command: last_command.to_string(),
        last_output,
        kern_boottime_command: kern_boottime_command.to_string(),
        kern_boottime_output,
    };
    validate_host_shutdown_observation(&observation)?;
    Ok(observation)
}

pub fn validate_host_shutdown_observation(
    observation: &AgentTokenHostShutdownObservation,
) -> Result<(), String> {
    if observation.contract != AGENT_TOKEN_HOST_SHUTDOWN_OBSERVATION_CONTRACT
        || observation.shutdown_at_unix_s != AGENT_TOKEN_HOST_SHUTDOWN_AT_UNIX_S
        || observation.reboot_at_unix_s != AGENT_TOKEN_HOST_REBOOT_AT_UNIX_S
        || observation.last_command != "/usr/bin/last reboot shutdown"
        || observation.kern_boottime_command != "/usr/sbin/sysctl -n kern.boottime"
        || observation.captured_at.trim().is_empty()
    {
        return Err("Host-shutdown observation identity differs".into());
    }
    if !(AGENT_TOKEN_HOST_SHUTDOWN_AT_UNIX_S
        ..AGENT_TOKEN_HOST_SHUTDOWN_AT_UNIX_S.saturating_add(60))
        .contains(&observation.interrupted_event_mtime_unix_s)
    {
        return Err("Host-shutdown observation event time differs".into());
    }
    let lines = observation.last_output.lines().collect::<Vec<_>>();
    if lines
        .first()
        .is_none_or(|line| !line.starts_with("reboot time") || !line.contains("Sat Aug 29 02:19"))
        || lines.get(1).is_none_or(|line| {
            !line.starts_with("shutdown time") || !line.contains("Sat Aug 29 02:14")
        })
    {
        return Err("Host-shutdown last(1) evidence differs".into());
    }
    if !observation
        .kern_boottime_output
        .contains(&format!("sec = {}", AGENT_TOKEN_HOST_REBOOT_AT_UNIX_S))
    {
        return Err("Host-shutdown kern.boottime evidence differs".into());
    }
    Ok(())
}

pub fn load_agent_token_host_shutdown_recovery_view(
    manifest: &AgentTokenCampaignManifest,
    source_schedule: &AgentTokenSchedule,
    campaign_dir: &Path,
    source_runs: &[AgentTokenRunSummary],
) -> Result<Option<AgentTokenHostShutdownRecoveryView>, String> {
    let selection_path = campaign_dir.join(AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_SELECTION_FILE);
    if !selection_path.exists() {
        return Ok(None);
    }
    require_regular_file(&selection_path, "host-shutdown recovery selection")?;
    let selection = read_json::<AgentTokenHostShutdownPairRecoverySelection>(
        &selection_path,
        "host-shutdown recovery selection",
    )?;
    validate_host_shutdown_selection_identity(&selection, manifest, source_schedule)?;

    let base = load_agent_token_infrastructure_recovery_view_with_additional_gap(
        manifest,
        source_schedule,
        campaign_dir,
        source_runs,
        Some(selection.source_pair_start_index),
    )?
    .ok_or_else(|| {
        "Host-shutdown recovery requires the prior infrastructure recovery".to_string()
    })?;

    let interruption = classify_host_shutdown_interruption(
        manifest,
        source_schedule,
        campaign_dir,
        AGENT_TOKEN_HOST_SHUTDOWN_PAIR_START_INDEX,
    )?;
    let expected_directory = format!("runs/{}", selection.interrupted_run_id);
    let expected_event_path = format!(
        "runs/{}/private/codex-events.raw.jsonl",
        selection.interrupted_run_id
    );
    if selection.interrupted_run_directory != expected_directory
        || selection.interrupted_artifacts != interruption.artifacts
        || selection.interrupted_event_path != expected_event_path
        || selection.interrupted_event_sha256 != interruption.event_sha256
        || selection.interrupted_event_mtime_unix_s != interruption.event_mtime_unix_s
        || selection.terminal_provider_event_observed
        || selection.run_summary_observed
    {
        return Err(
            "Host-shutdown interrupted evidence differs from its immutable inventory".into(),
        );
    }

    let observation_path = campaign_dir.join(portable_relative_path(&selection.host_observation)?);
    require_digest(
        &observation_path,
        &selection.host_observation_sha256,
        "host-shutdown observation",
    )?;
    let observation = read_json::<AgentTokenHostShutdownObservation>(
        &observation_path,
        "host-shutdown observation",
    )?;
    validate_host_shutdown_observation(&observation)?;
    if observation.interrupted_event_mtime_unix_s != interruption.event_mtime_unix_s {
        return Err("Host-shutdown observation is not linked to the interrupted event".into());
    }

    let pair = exact_authorized_pair(source_schedule)?;
    let replacement_entries = pair
        .iter()
        .map(|entry| {
            let mut replacement = entry.clone();
            replacement.run_id = host_shutdown_replacement_run_id(&entry.run_id);
            replacement
        })
        .collect::<Vec<_>>();
    let mut replacement_runs = Vec::new();
    let mut effective_paths = base.effective_run_summary_paths.clone();
    for (entry, artifact) in replacement_entries
        .iter()
        .zip(selection.replacement_runs.iter())
    {
        if artifact.run_id != entry.run_id {
            return Err("Host-shutdown replacement run identity differs".into());
        }
        let path = campaign_dir.join(portable_relative_path(&artifact.run_summary)?);
        require_digest(
            &path,
            &artifact.run_summary_sha256,
            "host-shutdown replacement run summary",
        )?;
        let run =
            read_json::<AgentTokenRunSummary>(&path, "host-shutdown replacement run summary")?;
        require_run_identity(&run, entry, manifest)?;
        require_valid_replacement(&run)?;
        validate_replacement_run_files(manifest, campaign_dir, &run, &path)?;
        effective_paths.insert(run.run_id.clone(), path);
        replacement_runs.push(run);
    }

    let source_pair_ids = pair
        .iter()
        .map(|entry| entry.run_id.as_str())
        .collect::<BTreeSet<_>>();
    if source_runs
        .iter()
        .any(|run| source_pair_ids.contains(run.run_id.as_str()))
    {
        return Err("Host-shutdown source pair unexpectedly contains a completed summary".into());
    }
    let mut effective_schedule = base.effective_schedule.clone();
    for (entry, replacement) in effective_schedule.entries
        [selection.source_pair_start_index..selection.source_pair_start_index + 2]
        .iter_mut()
        .zip(replacement_entries)
    {
        entry.run_id = replacement.run_id;
    }
    let mut effective_runs = base.effective_runs;
    effective_runs.extend(replacement_runs);
    let order = effective_schedule
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.run_id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    effective_runs.sort_by_key(|run| order.get(run.run_id.as_str()).copied());
    for (index, run) in effective_runs.iter().enumerate() {
        if effective_schedule
            .entries
            .get(index)
            .map(|entry| entry.run_id.as_str())
            != Some(run.run_id.as_str())
        {
            return Err("Host-shutdown effective runs are not an exact schedule prefix".into());
        }
    }

    Ok(Some(AgentTokenHostShutdownRecoveryView {
        effective_schedule,
        effective_runs,
        excluded_runs: base.excluded_runs,
        effective_run_summary_paths: effective_paths,
        excluded_run_summary_paths: base.excluded_run_summary_paths,
        infrastructure_selection: base.selection,
        selection,
    }))
}

pub(crate) fn validate_host_shutdown_selection_identity(
    selection: &AgentTokenHostShutdownPairRecoverySelection,
    manifest: &AgentTokenCampaignManifest,
    schedule: &AgentTokenSchedule,
) -> Result<(), String> {
    let pair = exact_authorized_pair(schedule)?;
    if selection.contract != AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_CONTRACT
        || selection.campaign_id != manifest.campaign_id
        || selection.campaign_id != AGENT_TOKEN_HOST_SHUTDOWN_CAMPAIGN_ID
        || selection.source_protocol_revision != manifest.protocol_revision
        || selection.policy_revision != AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_POLICY_REVISION
        || selection.source_pair_start_index != AGENT_TOKEN_HOST_SHUTDOWN_PAIR_START_INDEX
        || selection.workload_id != pair[0].workload_id
        || selection.attempt != pair[0].attempt
        || selection.source_schedule_run_ids
            != pair
                .iter()
                .map(|entry| entry.run_id.clone())
                .collect::<Vec<_>>()
        || selection.interrupted_run_id != AGENT_TOKEN_HOST_SHUTDOWN_INTERRUPTED_RUN_ID
        || selection.replacement_runs.len() != 2
        || selection.recovery_runner_sha256.trim().is_empty()
        || selection.reason != AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_REASON
        || selection.selected_at.trim().is_empty()
    {
        return Err("Host-shutdown recovery selection identity differs".into());
    }
    Ok(())
}

fn exact_authorized_pair(
    schedule: &AgentTokenSchedule,
) -> Result<&[AgentTokenScheduleEntry], String> {
    let pair = schedule
        .entries
        .get(
            AGENT_TOKEN_HOST_SHUTDOWN_PAIR_START_INDEX
                ..AGENT_TOKEN_HOST_SHUTDOWN_PAIR_START_INDEX + 2,
        )
        .ok_or_else(|| {
            "Host-shutdown authorized pair is outside the frozen schedule".to_string()
        })?;
    if pair[0].workload_id != "GD-05"
        || pair[0].attempt != 9
        || pair
            .iter()
            .any(|entry| entry.workload_id != "GD-05" || entry.attempt != 9)
        || pair.iter().map(|entry| entry.mode).collect::<BTreeSet<_>>()
            != BTreeSet::from([
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ])
    {
        return Err("Host-shutdown authorized pair identity differs".into());
    }
    Ok(pair)
}

fn require_valid_replacement(run: &AgentTokenRunSummary) -> Result<(), String> {
    if !run.valid_attempt
        || run.infrastructure_failure.is_some()
        || !run.invalid_reasons.is_empty()
        || run.usage.is_none()
        || !run.transcript.valid
        || !run.transcript.errors.is_empty()
    {
        return Err(format!(
            "Host-shutdown replacement run {} is not protocol-valid",
            run.run_id
        ));
    }
    let usage = run.usage.as_ref().expect("host recovery usage was checked");
    if usage.run_id != run.run_id
        || usage.workload_id != run.workload_id
        || usage.mode != run.mode
        || usage.accounting_profile != run.accounting_profile
    {
        return Err(format!(
            "Host-shutdown replacement run {} usage linkage differs",
            run.run_id
        ));
    }
    Ok(())
}

fn inventory_regular_files(root: &Path) -> Result<Vec<AgentTokenInterruptedArtifact>, String> {
    fn visit(
        root: &Path,
        directory: &Path,
        artifacts: &mut Vec<AgentTokenInterruptedArtifact>,
    ) -> Result<(), String> {
        let mut entries = fs::read_dir(directory)
            .map_err(|error| format!("Failed to inventory {}: {error}", directory.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Failed to inventory {}: {error}", directory.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "Host-shutdown interrupted evidence contains a symlink: {}",
                    path.display()
                ));
            }
            if metadata.is_dir() {
                visit(root, &path, artifacts)?;
            } else if metadata.is_file() {
                let bytes = fs::read(&path).map_err(|error| {
                    format!(
                        "Failed to read interrupted artifact {}: {error}",
                        path.display()
                    )
                })?;
                let relative = path.strip_prefix(root).map_err(|_| {
                    format!("Interrupted artifact escaped its root: {}", path.display())
                })?;
                artifacts.push(AgentTokenInterruptedArtifact {
                    path: relative_path_string(relative)?,
                    size_bytes: metadata.len(),
                    sha256: sha256_digest(&bytes),
                });
            } else {
                return Err(format!(
                    "Host-shutdown interrupted evidence contains a special file: {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    let mut artifacts = Vec::new();
    visit(root, root, &mut artifacts)?;
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(artifacts)
}

fn terminal_provider_event_observed(bytes: &[u8]) -> Result<bool, String> {
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let event = serde_json::from_slice::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Failed to decode interrupted event line {}: {error}",
                index + 1
            )
        })?;
        let kind = event.get("type").and_then(serde_json::Value::as_str);
        if matches!(kind, Some("turn.completed" | "turn.failed" | "error")) {
            return Ok(true);
        }
        if kind == Some("item.completed")
            && event
                .pointer("/item/type")
                .and_then(serde_json::Value::as_str)
                == Some("error")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn regular_file_mtime_s(path: &Path) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("Expected regular file: {}", path.display()));
    }
    metadata
        .modified()
        .map_err(|error| format!("Failed to read mtime for {}: {error}", path.display()))?
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("Invalid mtime for {}: {error}", path.display()))
}

fn command_output(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("Failed to execute {program}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Host-shutdown evidence command {program} failed with {}",
            output.status
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("Host-shutdown evidence command output is not UTF-8: {error}"))
}

fn relative_path_string(path: &Path) -> Result<String, String> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "Host-shutdown artifact path must be portable: {}",
            path.display()
        ));
    }
    Ok(path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn portable_relative_path(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "Host-shutdown recovery path must be portable and relative: {value:?}"
        ));
    }
    Ok(path.to_path_buf())
}

fn require_digest(path: &Path, expected: &str, label: &str) -> Result<(), String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Failed to read {label} {}: {error}", path.display()))?;
    let actual = sha256_digest(&bytes);
    if actual != expected && format!("sha256:{actual}") != expected {
        return Err(format!(
            "Host-shutdown {label} digest differs: expected {expected}, got {actual}"
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

fn require_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Failed to inspect {label} {}: {error}", path.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("{label} must be a directory: {}", path.display()));
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
    fn host_shutdown_observation_requires_the_exact_shutdown_and_reboot() {
        let valid = AgentTokenHostShutdownObservation {
            contract: AGENT_TOKEN_HOST_SHUTDOWN_OBSERVATION_CONTRACT.to_string(),
            captured_at: "2026-08-29T06:40:00+08:00".to_string(),
            shutdown_at_unix_s: AGENT_TOKEN_HOST_SHUTDOWN_AT_UNIX_S,
            reboot_at_unix_s: AGENT_TOKEN_HOST_REBOOT_AT_UNIX_S,
            interrupted_event_mtime_unix_s: AGENT_TOKEN_HOST_SHUTDOWN_AT_UNIX_S + 5,
            last_command: "/usr/bin/last reboot shutdown".to_string(),
            last_output: "reboot time Sat Aug 29 02:19\nshutdown time Sat Aug 29 02:14\n"
                .to_string(),
            kern_boottime_command: "/usr/sbin/sysctl -n kern.boottime".to_string(),
            kern_boottime_output: format!(
                "{{ sec = {}, usec = 783354 }} Sat Aug 29 02:19:32 2026\n",
                AGENT_TOKEN_HOST_REBOOT_AT_UNIX_S
            ),
        };
        assert!(validate_host_shutdown_observation(&valid).is_ok());
        let mut invalid = valid;
        invalid.reboot_at_unix_s += 1;
        assert!(validate_host_shutdown_observation(&invalid).is_err());
    }

    #[test]
    fn host_shutdown_replacement_ids_are_distinct() {
        assert_eq!(
            host_shutdown_replacement_run_id("campaign-b009-gd-05-git"),
            "campaign-b009-gd-05-git-host-shutdown-recovery-01"
        );
    }
}
