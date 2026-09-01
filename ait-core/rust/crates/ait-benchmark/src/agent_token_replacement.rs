use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::agent_token::{load_agent_token_run_summaries_with_allowed_missing, AgentTokenExecutor};
use crate::agent_token_runner::preflight_usage_from_normalized;

use crate::{
    build_agent_token_report, load_agent_token_host_shutdown_recovery_view,
    load_agent_token_infrastructure_recovery_view, load_agent_token_run_summaries, sha256_digest,
    AgentTokenCampaignManifest, AgentTokenEnvironment, AgentTokenExecutorPreflightReport,
    AgentTokenHostShutdownPairRecoveryRecord, AgentTokenHostShutdownPairRecoverySelection,
    AgentTokenInfrastructurePairRecoveryRecord, AgentTokenInfrastructurePairRecoverySelection,
    AgentTokenMode, AgentTokenRecoveredSpawnAdjudicationRecord, AgentTokenReport,
    AgentTokenRunAdjudication, AgentTokenRunManifest, AgentTokenRunSummary, AgentTokenSchedule,
    AgentTokenStatisticalReplacementRecord, AGENT_TOKEN_ADJUDICATOR_REVISION,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_PAIR_ADMISSION_POLICY,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_POLICY_REVISION,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_SELECTION_FILE,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_PAIR_ADMISSION_POLICY,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_POLICY_REVISION,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_SELECTION_FILE,
    AGENT_TOKEN_RECOVERED_SPAWN_PAIR_ADMISSION_POLICY, AGENT_TOKEN_RECOVERED_SPAWN_POLICY_REVISION,
    AGENT_TOKEN_RECOVERED_SPAWN_REASON, AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID,
};

pub const AGENT_TOKEN_REPLACEMENT_POLICY_REVISION: &str = "game-development-2026-08-27.29";
pub const AGENT_TOKEN_REPLACEMENT_SELECTION_CONTRACT: &str =
    "ait-agent-token-statistical-replacement/v1";
pub const AGENT_TOKEN_REPLACEMENT_PAIR_ADMISSION_POLICY: &str =
    "exact_protocol_valid_pair_with_owner_authorized_single_lane_statistical_replacement";
pub const AGENT_TOKEN_REPLACEMENT_SELECTION_FILE: &str = "statistical-replacement.json";
pub const AGENT_TOKEN_REPLACEMENT_CAMPAIGN_ID: &str = "game-v1-g56s-max-complete200-fx27-20260826";
pub const AGENT_TOKEN_REPLACED_RUN_ID: &str =
    "game-v1-g56s-max-complete200-fx27-20260826-b006-gd-05-ait";
pub const AGENT_TOKEN_REPLACEMENT_RUN_ID: &str =
    "game-v1-g56s-max-complete200-fx27-20260826-b006-gd-05-ait-replacement-01";
pub const AGENT_TOKEN_REPLACEMENT_REASON: &str =
    "Repository-owner-authorized transparent statistical replacement of the disclosed valid GD-05 AIT functional failure";
pub const AGENT_TOKEN_SPRINT_ON_REPLACEMENT_POLICY_REVISION: &str =
    "game-development-2026-08-29.36";
pub const AGENT_TOKEN_SPRINT_ON_REPLACEMENT_PAIR_ADMISSION_POLICY: &str =
    "exact_protocol_valid_pairs_with_transparent_infrastructure_and_host_shutdown_whole_pair_recovery_plus_digest_linked_recovered_spawn_adjudication_and_single_lane_statistical_replacement";
pub const AGENT_TOKEN_SPRINT_ON_REPLACEMENT_CAMPAIGN_ID: &str =
    "game-v1-g56s-max-sprint-on-natural-complete200-20260828";
pub const AGENT_TOKEN_SPRINT_ON_REPLACED_RUN_ID: &str =
    "game-v1-g56s-max-sprint-on-natural-complete200-20260828-b006-gd-05-ait";
pub const AGENT_TOKEN_SPRINT_ON_REPLACEMENT_RUN_ID: &str =
    "game-v1-g56s-max-sprint-on-natural-complete200-20260828-b006-gd-05-ait-replacement-01";
pub const AGENT_TOKEN_PROSPECTIVE_REPLACEMENT_REASON: &str =
    "Prospective manifest-declared replacement of the first protocol-valid unaccepted lane; one execution maximum with the raw failure retained";
pub const AGENT_TOKEN_PROSPECTIVE_REPLACEMENT_PAIR_ADMISSION_POLICY: &str =
    "exact_protocol_valid_pairs_with_manifest_declared_symmetric_first_functional_failure_single_lane_replacement";

#[derive(Clone, Debug)]
pub(crate) struct AgentTokenStatisticalReplacementAuthorization {
    pub campaign_id: String,
    pub source_run_id: String,
    pub replacement_run_id: String,
    pub policy_revision: String,
    pub pair_admission_policy: String,
    pub evaluation_mode: String,
    pub reason: String,
}

pub(crate) fn statistical_replacement_authorization(
    manifest: &AgentTokenCampaignManifest,
    source_run_id: &str,
) -> Result<AgentTokenStatisticalReplacementAuthorization, String> {
    let campaign_id = manifest.campaign_id.as_str();
    let authorization = if campaign_id == AGENT_TOKEN_REPLACEMENT_CAMPAIGN_ID
        && source_run_id == AGENT_TOKEN_REPLACED_RUN_ID
    {
        AgentTokenStatisticalReplacementAuthorization {
            campaign_id: AGENT_TOKEN_REPLACEMENT_CAMPAIGN_ID.to_string(),
            source_run_id: AGENT_TOKEN_REPLACED_RUN_ID.to_string(),
            replacement_run_id: AGENT_TOKEN_REPLACEMENT_RUN_ID.to_string(),
            policy_revision: AGENT_TOKEN_REPLACEMENT_POLICY_REVISION.to_string(),
            pair_admission_policy: AGENT_TOKEN_REPLACEMENT_PAIR_ADMISSION_POLICY.to_string(),
            evaluation_mode: "owner_authorized_statistical_replacement".to_string(),
            reason: AGENT_TOKEN_REPLACEMENT_REASON.to_string(),
        }
    } else if campaign_id == AGENT_TOKEN_SPRINT_ON_REPLACEMENT_CAMPAIGN_ID
        && source_run_id == AGENT_TOKEN_SPRINT_ON_REPLACED_RUN_ID
    {
        AgentTokenStatisticalReplacementAuthorization {
            campaign_id: AGENT_TOKEN_SPRINT_ON_REPLACEMENT_CAMPAIGN_ID.to_string(),
            source_run_id: AGENT_TOKEN_SPRINT_ON_REPLACED_RUN_ID.to_string(),
            replacement_run_id: AGENT_TOKEN_SPRINT_ON_REPLACEMENT_RUN_ID.to_string(),
            policy_revision: AGENT_TOKEN_SPRINT_ON_REPLACEMENT_POLICY_REVISION.to_string(),
            pair_admission_policy: AGENT_TOKEN_SPRINT_ON_REPLACEMENT_PAIR_ADMISSION_POLICY
                .to_string(),
            evaluation_mode: "owner_authorized_recovery_adjudication_and_statistical_replacement"
                .to_string(),
            reason: AGENT_TOKEN_REPLACEMENT_REASON.to_string(),
        }
    } else if manifest.protocol_revision == crate::AGENT_TOKEN_PROTOCOL_REVISION
        && manifest.campaign_scope == crate::AgentTokenCampaignScope::Complete
        && manifest.functional_replacement_policy
            == crate::AgentTokenFunctionalReplacementPolicy::FirstValidUnacceptedLaneOnce
        && source_run_id.starts_with(&format!("{}-", manifest.campaign_id))
        && !source_run_id.contains('/')
        && !source_run_id.contains("..")
    {
        AgentTokenStatisticalReplacementAuthorization {
            campaign_id: manifest.campaign_id.clone(),
            source_run_id: source_run_id.to_string(),
            replacement_run_id: format!("{source_run_id}-replacement-01"),
            policy_revision: crate::AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
            pair_admission_policy: AGENT_TOKEN_PROSPECTIVE_REPLACEMENT_PAIR_ADMISSION_POLICY
                .to_string(),
            evaluation_mode: "prospective_manifest_declared_statistical_replacement".to_string(),
            reason: AGENT_TOKEN_PROSPECTIVE_REPLACEMENT_REASON.to_string(),
        }
    } else {
        return Err(
            "Statistical replacement is not authorized by the frozen campaign manifest or an exact legacy policy"
                .to_string(),
        );
    };
    Ok(authorization)
}

pub(crate) fn first_valid_unaccepted_run_id<'a>(
    schedule: &AgentTokenSchedule,
    runs: &'a [AgentTokenRunSummary],
) -> Option<&'a str> {
    schedule.entries.iter().find_map(|entry| {
        runs.iter()
            .find(|run| run.run_id == entry.run_id)
            .filter(|run| run.valid_attempt && !run.accepted_equivalent)
            .map(|run| run.run_id.as_str())
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenStatisticalReplacementSelection {
    pub contract: String,
    pub campaign_id: String,
    pub policy_revision: String,
    pub source_run_id: String,
    pub source_run_summary_sha256: String,
    pub replacement_run_id: String,
    pub replacement_run_summary: String,
    pub replacement_run_summary_sha256: String,
    pub replacement_runner_sha256: String,
    pub reason: String,
    pub selected_at: String,
}

#[derive(Clone, Debug)]
pub struct AgentTokenCampaignStatisticalView {
    pub report: AgentTokenReport,
    pub effective_schedule: AgentTokenSchedule,
    pub effective_runs: Vec<AgentTokenRunSummary>,
    pub excluded_runs: Vec<AgentTokenRunSummary>,
    pub effective_run_summary_paths: BTreeMap<String, PathBuf>,
    pub excluded_run_summary_paths: BTreeMap<String, PathBuf>,
    pub selection: Option<AgentTokenStatisticalReplacementSelection>,
    pub infrastructure_recovery: Option<AgentTokenInfrastructurePairRecoverySelection>,
    pub host_shutdown_recovery: Option<AgentTokenHostShutdownPairRecoverySelection>,
}

pub fn load_agent_token_campaign_statistical_view(
    manifest: &AgentTokenCampaignManifest,
    source_schedule: &AgentTokenSchedule,
    campaign_dir: &Path,
) -> Result<AgentTokenCampaignStatisticalView, String> {
    load_agent_token_campaign_statistical_view_internal(
        manifest,
        source_schedule,
        campaign_dir,
        false,
    )
}

pub(crate) fn load_agent_token_campaign_statistical_view_allowing_host_shutdown_partial(
    manifest: &AgentTokenCampaignManifest,
    source_schedule: &AgentTokenSchedule,
    campaign_dir: &Path,
) -> Result<AgentTokenCampaignStatisticalView, String> {
    load_agent_token_campaign_statistical_view_internal(
        manifest,
        source_schedule,
        campaign_dir,
        true,
    )
}

fn load_agent_token_campaign_statistical_view_internal(
    manifest: &AgentTokenCampaignManifest,
    source_schedule: &AgentTokenSchedule,
    campaign_dir: &Path,
    allow_host_shutdown_partial: bool,
) -> Result<AgentTokenCampaignStatisticalView, String> {
    let selection_path = campaign_dir.join(AGENT_TOKEN_REPLACEMENT_SELECTION_FILE);
    let infrastructure_selection_path =
        campaign_dir.join(AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_SELECTION_FILE);
    let host_shutdown_selection_path =
        campaign_dir.join(AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_SELECTION_FILE);
    let source_runs = if allow_host_shutdown_partial || host_shutdown_selection_path.is_file() {
        load_agent_token_run_summaries_with_allowed_missing(
            campaign_dir,
            Some(crate::AGENT_TOKEN_HOST_SHUTDOWN_INTERRUPTED_RUN_ID),
        )?
    } else {
        load_agent_token_run_summaries(campaign_dir)?
    };
    if host_shutdown_selection_path.exists() && !infrastructure_selection_path.exists() {
        return Err(
            "Host-shutdown recovery requires the campaign's prior infrastructure recovery"
                .to_string(),
        );
    }
    if let Some(recovery) = load_agent_token_host_shutdown_recovery_view(
        manifest,
        source_schedule,
        campaign_dir,
        &source_runs,
    )? {
        let source_report = build_agent_token_report(manifest, source_schedule, &source_runs)?;
        let mut report = build_agent_token_report(
            manifest,
            &recovery.effective_schedule,
            &recovery.effective_runs,
        )?;
        report.source_protocol_claim_eligible = source_report.source_protocol_claim_eligible;
        report.source_protocol_blockers = source_report.blockers.clone();
        report.current_policy_revision =
            AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_POLICY_REVISION.to_string();
        report.current_policy_evaluation_mode =
            "owner_authorized_infrastructure_and_host_shutdown_whole_pair_recovery".to_string();
        report.current_policy_criteria_met = report.blockers.is_empty() && report.claim_eligible;
        report.current_policy_blockers = report.blockers.clone();
        report.claim_eligible = report.current_policy_criteria_met;
        report.executed_evidence_run_count = source_runs
            .len()
            .saturating_add(recovery.infrastructure_selection.replacement_runs.len())
            .saturating_add(recovery.selection.replacement_runs.len())
            .saturating_add(1);
        report.statistically_excluded_run_count = recovery.excluded_runs.len().saturating_add(1);
        report.pair_admission_policy =
            AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_PAIR_ADMISSION_POLICY.to_string();
        report.infrastructure_recovery_policy_revision =
            Some(AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_POLICY_REVISION.to_string());
        report.infrastructure_pair_recoveries = vec![AgentTokenInfrastructurePairRecoveryRecord {
            source_pair_start_index: recovery.infrastructure_selection.source_pair_start_index,
            workload_id: recovery.infrastructure_selection.workload_id.clone(),
            attempt: recovery.infrastructure_selection.attempt,
            source_schedule_run_ids: recovery
                .infrastructure_selection
                .source_schedule_run_ids
                .clone(),
            observed_source_run_ids: recovery
                .infrastructure_selection
                .observed_source_runs
                .iter()
                .map(|artifact| artifact.run_id.clone())
                .collect(),
            replacement_run_ids: recovery
                .infrastructure_selection
                .replacement_runs
                .iter()
                .map(|artifact| artifact.run_id.clone())
                .collect(),
            recovery_runner_sha256: recovery
                .infrastructure_selection
                .recovery_runner_sha256
                .clone(),
            reason: recovery.infrastructure_selection.reason.clone(),
        }];
        report.host_shutdown_recovery_policy_revision =
            Some(AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_POLICY_REVISION.to_string());
        report.host_shutdown_pair_recoveries = vec![AgentTokenHostShutdownPairRecoveryRecord {
            source_pair_start_index: recovery.selection.source_pair_start_index,
            workload_id: recovery.selection.workload_id.clone(),
            attempt: recovery.selection.attempt,
            source_schedule_run_ids: recovery.selection.source_schedule_run_ids.clone(),
            interrupted_run_id: recovery.selection.interrupted_run_id.clone(),
            interrupted_event_sha256: recovery.selection.interrupted_event_sha256.clone(),
            interrupted_event_mtime_unix_s: recovery.selection.interrupted_event_mtime_unix_s,
            interrupted_artifact_count: recovery.selection.interrupted_artifacts.len(),
            host_observation_sha256: recovery.selection.host_observation_sha256.clone(),
            replacement_run_ids: recovery
                .selection
                .replacement_runs
                .iter()
                .map(|artifact| artifact.run_id.clone())
                .collect(),
            recovery_runner_sha256: recovery.selection.recovery_runner_sha256.clone(),
            reason: recovery.selection.reason.clone(),
        }];
        if let Some(adjudication) = recovered_spawn_adjudication_record(campaign_dir)? {
            report.current_policy_revision =
                AGENT_TOKEN_RECOVERED_SPAWN_POLICY_REVISION.to_string();
            report.current_policy_evaluation_mode =
                "owner_authorized_recovery_with_digest_linked_recovered_spawn_adjudication"
                    .to_string();
            report.pair_admission_policy =
                AGENT_TOKEN_RECOVERED_SPAWN_PAIR_ADMISSION_POLICY.to_string();
            report.recovered_spawn_policy_revision =
                Some(AGENT_TOKEN_RECOVERED_SPAWN_POLICY_REVISION.to_string());
            report.recovered_spawn_adjudications = vec![adjudication];
            report.limitations.push(format!(
                "Raw run {AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID} remains append-only with its original spawn-failure classification. Its digest-linked successor adjudication counts the recovered retry and all provider tokens as measured agent behavior; no lane was re-executed."
            ));
        }
        report.limitations.push(format!(
            "The evidenced host shutdown interrupted {} without a terminal provider event or run summary. Its {} immutable artifacts remain inventoried and excluded; the complete {} attempt {} pair was re-executed once under the disclosed host-shutdown recovery policy.",
            recovery.selection.interrupted_run_id,
            recovery.selection.interrupted_artifacts.len(),
            recovery.selection.workload_id,
            recovery.selection.attempt,
        ));
        let view = AgentTokenCampaignStatisticalView {
            report,
            effective_schedule: recovery.effective_schedule,
            effective_runs: recovery.effective_runs,
            excluded_runs: recovery.excluded_runs,
            effective_run_summary_paths: recovery.effective_run_summary_paths,
            excluded_run_summary_paths: recovery.excluded_run_summary_paths,
            selection: None,
            infrastructure_recovery: Some(recovery.infrastructure_selection),
            host_shutdown_recovery: Some(recovery.selection),
        };
        return apply_optional_statistical_replacement(manifest, campaign_dir, view);
    }
    if let Some(recovery) = load_agent_token_infrastructure_recovery_view(
        manifest,
        source_schedule,
        campaign_dir,
        &source_runs,
    )? {
        let source_report = build_agent_token_report(manifest, source_schedule, &source_runs)?;
        let mut report = build_agent_token_report(
            manifest,
            &recovery.effective_schedule,
            &recovery.effective_runs,
        )?;
        report.source_protocol_claim_eligible = source_report.source_protocol_claim_eligible;
        report.source_protocol_blockers = source_report.blockers.clone();
        report.current_policy_revision =
            AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_POLICY_REVISION.to_string();
        report.current_policy_evaluation_mode =
            "owner_authorized_whole_pair_infrastructure_recovery".to_string();
        report.current_policy_criteria_met = report.blockers.is_empty() && report.claim_eligible;
        report.current_policy_blockers = report.blockers.clone();
        report.claim_eligible = report.current_policy_criteria_met;
        report.executed_evidence_run_count = source_runs
            .len()
            .saturating_add(recovery.selection.replacement_runs.len());
        report.statistically_excluded_run_count = recovery.excluded_runs.len();
        report.pair_admission_policy =
            AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_PAIR_ADMISSION_POLICY.to_string();
        report.infrastructure_recovery_policy_revision =
            Some(AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_POLICY_REVISION.to_string());
        report.infrastructure_pair_recoveries = vec![AgentTokenInfrastructurePairRecoveryRecord {
            source_pair_start_index: recovery.selection.source_pair_start_index,
            workload_id: recovery.selection.workload_id.clone(),
            attempt: recovery.selection.attempt,
            source_schedule_run_ids: recovery.selection.source_schedule_run_ids.clone(),
            observed_source_run_ids: recovery
                .selection
                .observed_source_runs
                .iter()
                .map(|artifact| artifact.run_id.clone())
                .collect(),
            replacement_run_ids: recovery
                .selection
                .replacement_runs
                .iter()
                .map(|artifact| artifact.run_id.clone())
                .collect(),
            recovery_runner_sha256: recovery.selection.recovery_runner_sha256.clone(),
            reason: recovery.selection.reason.clone(),
        }];
        if let Some(adjudication) = recovered_spawn_adjudication_record(campaign_dir)? {
            report.current_policy_revision =
                AGENT_TOKEN_RECOVERED_SPAWN_POLICY_REVISION.to_string();
            report.current_policy_evaluation_mode =
                "owner_authorized_recovery_with_digest_linked_recovered_spawn_adjudication"
                    .to_string();
            report.pair_admission_policy =
                AGENT_TOKEN_RECOVERED_SPAWN_PAIR_ADMISSION_POLICY.to_string();
            report.recovered_spawn_policy_revision =
                Some(AGENT_TOKEN_RECOVERED_SPAWN_POLICY_REVISION.to_string());
            report.recovered_spawn_adjudications = vec![adjudication];
            report.limitations.push(format!(
                "Raw run {AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID} remains append-only with its original spawn-failure classification. Its digest-linked successor adjudication counts the recovered retry and all provider tokens as measured agent behavior; no lane was re-executed."
            ));
        }
        report.limitations.push(format!(
            "A recognized executor infrastructure failure contaminated the {} attempt {} pair. The observed source lane(s) remain append-only and excluded; both same-pinned lanes were re-executed once under the disclosed whole-pair recovery policy.",
            recovery.selection.workload_id, recovery.selection.attempt
        ));
        let view = AgentTokenCampaignStatisticalView {
            report,
            effective_schedule: recovery.effective_schedule,
            effective_runs: recovery.effective_runs,
            excluded_runs: recovery.excluded_runs,
            effective_run_summary_paths: recovery.effective_run_summary_paths,
            excluded_run_summary_paths: recovery.excluded_run_summary_paths,
            selection: None,
            infrastructure_recovery: Some(recovery.selection),
            host_shutdown_recovery: None,
        };
        return apply_optional_statistical_replacement(manifest, campaign_dir, view);
    }
    if !selection_path.exists() {
        let report = build_agent_token_report(manifest, source_schedule, &source_runs)?;
        let effective_run_summary_paths = source_runs
            .iter()
            .map(|run| {
                (
                    run.run_id.clone(),
                    campaign_dir
                        .join("runs")
                        .join(&run.run_id)
                        .join("run-summary.json"),
                )
            })
            .collect();
        return Ok(AgentTokenCampaignStatisticalView {
            report,
            effective_schedule: source_schedule.clone(),
            effective_runs: source_runs,
            excluded_runs: Vec::new(),
            effective_run_summary_paths,
            excluded_run_summary_paths: BTreeMap::new(),
            selection: None,
            infrastructure_recovery: None,
            host_shutdown_recovery: None,
        });
    }
    let report = build_agent_token_report(manifest, source_schedule, &source_runs)?;
    let effective_run_summary_paths = source_runs
        .iter()
        .map(|run| {
            (
                run.run_id.clone(),
                campaign_dir
                    .join("runs")
                    .join(&run.run_id)
                    .join("run-summary.json"),
            )
        })
        .collect();
    apply_optional_statistical_replacement(
        manifest,
        campaign_dir,
        AgentTokenCampaignStatisticalView {
            report,
            effective_schedule: source_schedule.clone(),
            effective_runs: source_runs,
            excluded_runs: Vec::new(),
            effective_run_summary_paths,
            excluded_run_summary_paths: BTreeMap::new(),
            selection: None,
            infrastructure_recovery: None,
            host_shutdown_recovery: None,
        },
    )
}

fn apply_optional_statistical_replacement(
    manifest: &AgentTokenCampaignManifest,
    campaign_dir: &Path,
    mut base: AgentTokenCampaignStatisticalView,
) -> Result<AgentTokenCampaignStatisticalView, String> {
    let selection_path = campaign_dir.join(AGENT_TOKEN_REPLACEMENT_SELECTION_FILE);
    if !selection_path.exists() {
        return Ok(base);
    }
    require_regular_file(&selection_path, "statistical replacement selection")?;
    let selection = read_json::<AgentTokenStatisticalReplacementSelection>(
        &selection_path,
        "statistical replacement selection",
    )?;
    validate_selection_identity(&selection, manifest)?;
    let authorization = statistical_replacement_authorization(manifest, &selection.source_run_id)?;

    if manifest.functional_replacement_policy
        == crate::AgentTokenFunctionalReplacementPolicy::FirstValidUnacceptedLaneOnce
        && first_valid_unaccepted_run_id(&base.effective_schedule, &base.effective_runs)
            != Some(selection.source_run_id.as_str())
    {
        return Err(
            "Prospective statistical replacement must target the first valid unaccepted lane in frozen schedule order"
                .to_string(),
        );
    }

    let source_index = base
        .effective_runs
        .iter()
        .position(|run| run.run_id == selection.source_run_id)
        .ok_or_else(|| {
            format!(
                "Statistical replacement source run {} is absent from the effective campaign",
                selection.source_run_id
            )
        })?;
    let source_run = base.effective_runs[source_index].clone();
    if !source_run.valid_attempt || source_run.accepted_equivalent {
        return Err(format!(
            "Statistical replacement source run {} must be valid and unaccepted",
            source_run.run_id
        ));
    }
    let source_summary_path = base
        .effective_run_summary_paths
        .get(&source_run.run_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "Statistical replacement source run {} has no effective summary path",
                source_run.run_id
            )
        })?;
    require_digest(
        &source_summary_path,
        &selection.source_run_summary_sha256,
        "statistical replacement source run summary",
    )?;

    let replacement_relative = portable_relative_path(&selection.replacement_run_summary)?;
    let expected_relative = PathBuf::from("statistical-replacements")
        .join("replacement-0001")
        .join("runs")
        .join(&selection.replacement_run_id)
        .join("run-summary.json");
    if replacement_relative != expected_relative {
        return Err(format!(
            "Statistical replacement run-summary path must be {}, got {}",
            expected_relative.display(),
            replacement_relative.display()
        ));
    }
    let replacement_summary_path = campaign_dir.join(&replacement_relative);
    require_regular_file(
        &replacement_summary_path,
        "statistical replacement run summary",
    )?;
    require_digest(
        &replacement_summary_path,
        &selection.replacement_run_summary_sha256,
        "statistical replacement run summary",
    )?;
    let replacement_run = read_json::<AgentTokenRunSummary>(
        &replacement_summary_path,
        "statistical replacement run summary",
    )?;
    validate_replacement_run(
        manifest,
        campaign_dir,
        &source_run,
        &replacement_run,
        &selection,
    )?;

    let schedule_entry = base
        .effective_schedule
        .entries
        .iter_mut()
        .find(|entry| entry.run_id == source_run.run_id)
        .ok_or_else(|| {
            format!(
                "Statistical replacement source run {} is absent from the effective schedule",
                source_run.run_id
            )
        })?;
    schedule_entry.run_id = replacement_run.run_id.clone();
    base.effective_runs[source_index] = replacement_run.clone();
    let order = base
        .effective_schedule
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.run_id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    base.effective_runs
        .sort_by_key(|run| order.get(run.run_id.as_str()).copied());

    let base_report = base.report;
    let mut report =
        build_agent_token_report(manifest, &base.effective_schedule, &base.effective_runs)?;
    report.source_protocol_claim_eligible = base_report.source_protocol_claim_eligible;
    report.source_protocol_blockers = base_report.source_protocol_blockers;
    report.current_policy_revision = authorization.policy_revision.to_string();
    report.current_policy_evaluation_mode = authorization.evaluation_mode.to_string();
    report.current_policy_criteria_met = report.blockers.is_empty() && report.claim_eligible;
    report.current_policy_blockers = report.blockers.clone();
    report.claim_eligible = report.current_policy_criteria_met;
    report.executed_evidence_run_count = base_report.executed_evidence_run_count.saturating_add(1);
    report.statistically_excluded_run_count = base_report
        .statistically_excluded_run_count
        .saturating_add(1);
    report.replacement_policy_revision = Some(authorization.policy_revision.to_string());
    report.pair_admission_policy = authorization.pair_admission_policy.to_string();
    report.statistical_replacements = vec![AgentTokenStatisticalReplacementRecord {
        source_run_id: selection.source_run_id.clone(),
        replacement_run_id: selection.replacement_run_id.clone(),
        source_run_summary_sha256: selection.source_run_summary_sha256.clone(),
        replacement_run_summary_sha256: selection.replacement_run_summary_sha256.clone(),
        replacement_runner_sha256: selection.replacement_runner_sha256.clone(),
        reason: selection.reason.clone(),
    }];
    report.infrastructure_recovery_policy_revision =
        base_report.infrastructure_recovery_policy_revision;
    report.infrastructure_pair_recoveries = base_report.infrastructure_pair_recoveries;
    report.host_shutdown_recovery_policy_revision =
        base_report.host_shutdown_recovery_policy_revision;
    report.host_shutdown_pair_recoveries = base_report.host_shutdown_pair_recoveries;
    report.recovered_spawn_policy_revision = base_report.recovered_spawn_policy_revision;
    report.recovered_spawn_adjudications = base_report.recovered_spawn_adjudications;
    report.limitations = base_report.limitations;
    report.limitations.push(format!(
        "The disclosed valid {} {} functional failure remains in append-only evidence but is excluded from effective statistics under the single-lane replacement policy declared for this campaign. The one same-pinned replacement must pass every original admission gate; a failed replacement never activates.",
        source_run.workload_id,
        source_run.mode.as_str(),
    ));

    base.effective_run_summary_paths.remove(&source_run.run_id);
    base.effective_run_summary_paths
        .insert(replacement_run.run_id.clone(), replacement_summary_path);
    base.excluded_run_summary_paths
        .insert(source_run.run_id.clone(), source_summary_path);
    base.excluded_runs.push(source_run);
    base.report = report;
    base.selection = Some(selection);
    Ok(base)
}

pub(crate) fn validate_selection_identity(
    selection: &AgentTokenStatisticalReplacementSelection,
    manifest: &AgentTokenCampaignManifest,
) -> Result<(), String> {
    let authorization = statistical_replacement_authorization(manifest, &selection.source_run_id)?;
    if selection.contract != AGENT_TOKEN_REPLACEMENT_SELECTION_CONTRACT
        || selection.campaign_id != manifest.campaign_id
        || selection.campaign_id != authorization.campaign_id
        || selection.source_run_id != authorization.source_run_id
        || selection.policy_revision != authorization.policy_revision
        || selection.replacement_run_id != authorization.replacement_run_id
        || selection.reason != authorization.reason
        || selection.selected_at.trim().is_empty()
    {
        return Err("Statistical replacement selection identity differs from its frozen authorization contract".to_string());
    }
    for (label, digest) in [
        (
            "source run summary",
            selection.source_run_summary_sha256.as_str(),
        ),
        (
            "replacement run summary",
            selection.replacement_run_summary_sha256.as_str(),
        ),
        (
            "replacement runner",
            selection.replacement_runner_sha256.as_str(),
        ),
    ] {
        let value = digest.strip_prefix("sha256:").unwrap_or(digest);
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!(
                "Statistical replacement {label} SHA-256 is malformed"
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_replacement_run(
    manifest: &AgentTokenCampaignManifest,
    campaign_dir: &Path,
    source: &AgentTokenRunSummary,
    replacement: &AgentTokenRunSummary,
    selection: &AgentTokenStatisticalReplacementSelection,
) -> Result<(), String> {
    if replacement.run_id != selection.replacement_run_id
        || replacement.campaign_id != source.campaign_id
        || replacement.campaign_id != manifest.campaign_id
        || replacement.workload_id != source.workload_id
        || replacement.mode != source.mode
        || replacement.accounting_profile != source.accounting_profile
        || replacement.attempt != source.attempt
        || replacement.block_index != source.block_index
        || replacement.randomized_order != source.randomized_order
        || replacement.initial_content_digest != source.initial_content_digest
    {
        return Err(
            "Statistical replacement run identity differs from its source lane".to_string(),
        );
    }
    if !replacement.valid_attempt
        || !replacement.accepted_equivalent
        || !replacement.evaluator_accepted
        || replacement.browser.status != "passed"
        || !replacement.workflow_closed
        || replacement.infrastructure_failure.is_some()
        || !replacement.invalid_reasons.is_empty()
        || !replacement.failure_reasons.is_empty()
        || replacement.usage.is_none()
        || !replacement.transcript.valid
        || !replacement.transcript.errors.is_empty()
    {
        return Err(format!(
            "Statistical replacement run {} did not pass every admission gate",
            replacement.run_id
        ));
    }
    let usage = replacement.usage.as_ref().expect("usage was checked");
    if usage.run_id != replacement.run_id
        || usage.workload_id != replacement.workload_id
        || usage.mode != replacement.mode
        || usage.accounting_profile != replacement.accounting_profile
        || usage.model_provider != manifest.model.provider
        || usage.model_id != manifest.model.model_id
        || usage.model_revision != manifest.model.model_revision
        || usage.reasoning_effort != manifest.model.reasoning_effort
    {
        return Err("Statistical replacement provider-usage linkage differs".to_string());
    }

    let source_dir = campaign_dir.join("runs").join(&source.run_id);
    let replacement_dir = campaign_dir
        .join("statistical-replacements/replacement-0001/runs")
        .join(&replacement.run_id);
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
            &replacement_dir.join(required),
            &format!("statistical replacement {required}"),
        )?;
    }
    if manifest.runtime.executor == AgentTokenExecutor::Claude {
        let raw_events = replacement_dir.join("private/codex-events.raw.jsonl");
        require_regular_file(
            &raw_events,
            "statistical replacement private Claude model-purity evidence",
        )?;
        let imported = crate::agent_token::import_claude_usage_with_outcome(
            &raw_events,
            &replacement.run_id,
            &replacement.workload_id,
            replacement.mode,
            replacement.accounting_profile,
            &manifest.model,
            manifest.claude_model_admission,
        )?;
        if replacement.usage.as_ref() != Some(&imported.usage)
            || imported.provider_refusal
            || imported.provider_stop_reason != "end_turn"
            || replacement.provider_refusal
            || replacement.provider_stop_reason.as_deref() != Some("end_turn")
        {
            return Err(
                "Statistical replacement private Claude model-purity or terminal outcome evidence differs"
                    .to_string(),
            );
        }
    }
    for identical in ["campaign-manifest.json", "fixture-manifest.json"] {
        if fs::read(source_dir.join(identical)).map_err(|error| {
            format!("Failed to read source replacement evidence {identical}: {error}")
        })? != fs::read(replacement_dir.join(identical))
            .map_err(|error| format!("Failed to read replacement evidence {identical}: {error}"))?
        {
            return Err(format!(
                "Statistical replacement {identical} differs from the source lane"
            ));
        }
    }
    let source_prompt = fs::read(source_dir.join("prompt.txt"))
        .map_err(|error| format!("Failed to read source replacement prompt: {error}"))?;
    let replacement_prompt = fs::read(replacement_dir.join("prompt.txt"))
        .map_err(|error| format!("Failed to read statistical replacement prompt: {error}"))?;
    require_exact_run_identity_substitution(
        &source_prompt,
        &replacement_prompt,
        &source.run_id,
        &replacement.run_id,
        "statistical replacement prompt",
    )?;

    let source_environment = read_json::<AgentTokenEnvironment>(
        &source_dir.join("environment.json"),
        "source environment",
    )?;
    let replacement_environment = read_json::<AgentTokenEnvironment>(
        &replacement_dir.join("environment.json"),
        "replacement environment",
    )?;
    if !environments_match(&source_environment, &replacement_environment) {
        return Err("Statistical replacement environment differs from the source lane".to_string());
    }
    let source_manifest = read_json::<AgentTokenRunManifest>(
        &source_dir.join("run-manifest.json"),
        "source run manifest",
    )?;
    let replacement_manifest = read_json::<AgentTokenRunManifest>(
        &replacement_dir.join("run-manifest.json"),
        "replacement run manifest",
    )?;
    let expected_sprint_item_ref = source_manifest
        .sprint_item_ref
        .as_ref()
        .map(|value| value.replace(&source.run_id, &replacement.run_id));
    if source_manifest.run_id != source.run_id
        || replacement_manifest.run_id != replacement.run_id
        || source_manifest.sprint_card_path != replacement_manifest.sprint_card_path
        || expected_sprint_item_ref != replacement_manifest.sprint_item_ref
        || source_manifest.measured_prompt_digest != sha256_digest(&source_prompt)
        || replacement_manifest.measured_prompt_digest != sha256_digest(&replacement_prompt)
        || source_manifest.fixture_revision != replacement_manifest.fixture_revision
        || source_manifest.fixture_content_digest != replacement_manifest.fixture_content_digest
        || source_manifest.shared_task_prompt_digest
            != replacement_manifest.shared_task_prompt_digest
        || source_manifest.network_policy != replacement_manifest.network_policy
        || source_manifest.tool_policy != replacement_manifest.tool_policy
        || source_manifest.codex_permission_profile != replacement_manifest.codex_permission_profile
        || source_manifest.codex_permission_profile_parent
            != replacement_manifest.codex_permission_profile_parent
        || source_manifest.benchmark_enabled_feature_overrides
            != replacement_manifest.benchmark_enabled_feature_overrides
        || source_manifest.benchmark_disabled_feature_overrides
            != replacement_manifest.benchmark_disabled_feature_overrides
        || source_manifest.project_document_loading != replacement_manifest.project_document_loading
        || source_manifest.project_doc_max_bytes != replacement_manifest.project_doc_max_bytes
        || source_manifest.workflow_mode != replacement_manifest.workflow_mode
        || source_manifest.sprint_mode != replacement_manifest.sprint_mode
        || source_manifest.ait_server_allowed != replacement_manifest.ait_server_allowed
    {
        return Err(
            "Statistical replacement run-manifest pins differ from the source lane".to_string(),
        );
    }

    let replacement_root = campaign_dir.join("statistical-replacements/replacement-0001");
    require_digest(
        &replacement_root.join("replacement-runner"),
        &selection.replacement_runner_sha256,
        "statistical replacement runner",
    )?;
    let preflight = read_json::<AgentTokenExecutorPreflightReport>(
        &replacement_root.join("executor-preflight-report.json"),
        "replacement executor preflight",
    )?;
    if !preflight.passed || !preflight.failure_reasons.is_empty() {
        return Err("Statistical replacement executor preflight did not pass".to_string());
    }
    if manifest.runtime.executor == AgentTokenExecutor::Claude {
        let preflight_raw = replacement_root.join("private/executor-preflight-events.raw.jsonl");
        require_regular_file(
            &preflight_raw,
            "statistical replacement private Claude preflight model-purity evidence",
        )?;
        let imported = crate::agent_token::import_claude_usage_with_outcome(
            &preflight_raw,
            &format!("{}-executor-preflight", manifest.campaign_id),
            "executor-preflight",
            AgentTokenMode::GitLinearSingleSession,
            manifest.accounting_profile,
            &manifest.model,
            manifest.claude_model_admission,
        )?;
        if imported.provider_refusal
            || imported.provider_stop_reason != "end_turn"
            || preflight.usage.as_ref() != Some(&preflight_usage_from_normalized(&imported.usage))
        {
            return Err(
                "Statistical replacement private Claude preflight model-purity evidence differs"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn require_exact_run_identity_substitution(
    source: &[u8],
    replacement: &[u8],
    source_run_id: &str,
    replacement_run_id: &str,
    label: &str,
) -> Result<(), String> {
    let source = std::str::from_utf8(source)
        .map_err(|error| format!("{label} source is not UTF-8: {error}"))?;
    let replacement = std::str::from_utf8(replacement)
        .map_err(|error| format!("{label} replacement is not UTF-8: {error}"))?;
    let expected = source.replace(source_run_id, replacement_run_id);
    if replacement != expected {
        return Err(format!(
            "{label} differs beyond the exact authorized run-ID substitution"
        ));
    }
    Ok(())
}

fn environments_match(left: &AgentTokenEnvironment, right: &AgentTokenEnvironment) -> bool {
    left.contract == right.contract
        && left.os == right.os
        && left.architecture == right.architecture
        && left.codex_version == right.codex_version
        && left.ait_version == right.ait_version
        && left.git_version == right.git_version
        && left.node_version == right.node_version
        && left.browser_version == right.browser_version
        && left.workflow_mode == right.workflow_mode
        && left.sprint_mode == right.sprint_mode
        && left.ait_server_connected == right.ait_server_connected
        && left.network_policy == right.network_policy
        && left.codex_permission_profile == right.codex_permission_profile
        && left.codex_permission_profile_parent == right.codex_permission_profile_parent
        && left.cache_class == right.cache_class
        && left.benchmark_enabled_feature_overrides == right.benchmark_enabled_feature_overrides
        && left.benchmark_disabled_feature_overrides == right.benchmark_disabled_feature_overrides
        && left.project_doc_max_bytes == right.project_doc_max_bytes
}

fn recovered_spawn_adjudication_record(
    campaign_dir: &Path,
) -> Result<Option<AgentTokenRecoveredSpawnAdjudicationRecord>, String> {
    let adjudication_path = campaign_dir
        .join("adjudications")
        .join(format!("{AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID}.json"));
    if !adjudication_path.exists() {
        return Ok(None);
    }
    require_regular_file(&adjudication_path, "recovered-spawn adjudication")?;
    let adjudication =
        read_json::<AgentTokenRunAdjudication>(&adjudication_path, "recovered-spawn adjudication")?;
    let source_path = campaign_dir
        .join("runs")
        .join(AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID)
        .join("run-summary.json");
    let source =
        read_json::<AgentTokenRunSummary>(&source_path, "recovered-spawn raw run summary")?;
    require_digest(
        &source_path,
        &adjudication.source_run_summary_sha256,
        "recovered-spawn raw run summary",
    )?;
    if adjudication.run_id != AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID
        || adjudication.adjudicator_revision != AGENT_TOKEN_ADJUDICATOR_REVISION
        || adjudication.reason != AGENT_TOKEN_RECOVERED_SPAWN_REASON
        || source.run_id != adjudication.run_id
        || source.infrastructure_failure.as_deref() != Some("codex_tool_process_spawn_failure")
        || !adjudication.effective_summary.valid_attempt
        || adjudication
            .effective_summary
            .infrastructure_failure
            .is_some()
    {
        return Err("Recovered-spawn report disclosure differs from its exact adjudication".into());
    }
    Ok(Some(AgentTokenRecoveredSpawnAdjudicationRecord {
        run_id: adjudication.run_id,
        source_run_summary_sha256: adjudication.source_run_summary_sha256,
        adjudicator_revision: adjudication.adjudicator_revision,
        source_infrastructure_failure: source
            .infrastructure_failure
            .expect("recovered-spawn source failure was checked"),
        reason: adjudication.reason,
    }))
}

fn portable_relative_path(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "Statistical replacement path must be a portable relative path: {value:?}"
        ));
    }
    Ok(path)
}

fn require_regular_file(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Failed to inspect {label} {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{label} is not a regular file: {}", path.display()));
    }
    Ok(())
}

fn require_digest(path: &Path, expected: &str, label: &str) -> Result<(), String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Failed to read {label} {}: {error}", path.display()))?;
    let observed = sha256_digest(&bytes);
    let normalized = if expected.starts_with("sha256:") {
        expected.to_string()
    } else {
        format!("sha256:{expected}")
    };
    if observed != normalized {
        return Err(format!(
            "{label} digest differs: expected {normalized}, got {observed}"
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
    use crate::agent_token::AgentTokenExecutor;
    use crate::{
        AgentTokenAccountingProfile, AgentTokenCampaignScope, AgentTokenMode, AgentTokenModelPin,
        AgentTokenRuntime,
    };

    fn authorized_manifest() -> AgentTokenCampaignManifest {
        AgentTokenCampaignManifest {
            contract: crate::AGENT_TOKEN_CAMPAIGN_CONTRACT.to_string(),
            campaign_id: AGENT_TOKEN_REPLACEMENT_CAMPAIGN_ID.to_string(),
            protocol_revision: crate::AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION
                .to_string(),
            campaign_scope: AgentTokenCampaignScope::Complete,
            accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
            ait_sprint_mode: crate::AgentTokenAitSprintMode::Off,
            ait_edit_root_mode: crate::agent_token::AgentTokenAitEditRootMode::Explicit,
            git_worktree_mode: crate::AgentTokenGitWorktreeMode::AgentManaged,
            claude_model_admission: crate::agent_token::AgentTokenClaudeModelAdmission::Strict,
            functional_replacement_policy: crate::AgentTokenFunctionalReplacementPolicy::None,
            seed: 202_608_264,
            attempts_per_cell: 20,
            workload_ids: ["GD-01", "GD-02", "GD-03", "GD-04", "GD-05"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            modes: vec![
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ],
            model: AgentTokenModelPin {
                provider: "openai".to_string(),
                model_id: "gpt-5.6-sol".to_string(),
                model_revision: "gpt-5.6-sol-provider-alias-observed-2026-08-23".to_string(),
                reasoning_effort: "max".to_string(),
            },
            runtime: AgentTokenRuntime {
                executor: AgentTokenExecutor::Codex,
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
            cache_class: "provider_default_fresh_ephemeral_session".to_string(),
            network_policy: "disabled_except_loopback".to_string(),
            tool_policy: "codex_shell_only".to_string(),
            bootstrap_resamples: 2_000,
            limitations: Vec::new(),
        }
    }

    fn authorized_selection() -> AgentTokenStatisticalReplacementSelection {
        AgentTokenStatisticalReplacementSelection {
            contract: AGENT_TOKEN_REPLACEMENT_SELECTION_CONTRACT.to_string(),
            campaign_id: AGENT_TOKEN_REPLACEMENT_CAMPAIGN_ID.to_string(),
            policy_revision: AGENT_TOKEN_REPLACEMENT_POLICY_REVISION.to_string(),
            source_run_id: AGENT_TOKEN_REPLACED_RUN_ID.to_string(),
            source_run_summary_sha256: format!("sha256:{}", "1".repeat(64)),
            replacement_run_id: AGENT_TOKEN_REPLACEMENT_RUN_ID.to_string(),
            replacement_run_summary:
                "statistical-replacements/replacement-0001/runs/replacement/run-summary.json"
                    .to_string(),
            replacement_run_summary_sha256: format!("sha256:{}", "2".repeat(64)),
            replacement_runner_sha256: format!("sha256:{}", "3".repeat(64)),
            reason: AGENT_TOKEN_REPLACEMENT_REASON.to_string(),
            selected_at: "2026-08-27T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn replacement_selection_is_exact_and_fail_closed() {
        let manifest = authorized_manifest();
        let selection = authorized_selection();
        validate_selection_identity(&selection, &manifest).unwrap();

        let mut wrong_source = selection.clone();
        wrong_source.source_run_id.push_str("-retry");
        assert!(validate_selection_identity(&wrong_source, &manifest).is_err());

        let mut wrong_reason = selection.clone();
        wrong_reason.reason = "discard a failure".to_string();
        assert!(validate_selection_identity(&wrong_reason, &manifest).is_err());

        let mut malformed_digest = selection;
        malformed_digest.replacement_runner_sha256 = "sha256:not-a-digest".to_string();
        assert!(validate_selection_identity(&malformed_digest, &manifest).is_err());
    }

    #[test]
    fn sprint_on_replacement_authorization_is_exact_and_distinct() {
        let mut manifest = authorized_manifest();
        manifest.campaign_id = AGENT_TOKEN_SPRINT_ON_REPLACEMENT_CAMPAIGN_ID.to_string();
        manifest.ait_sprint_mode = crate::AgentTokenAitSprintMode::On;
        let authorization =
            statistical_replacement_authorization(&manifest, AGENT_TOKEN_SPRINT_ON_REPLACED_RUN_ID)
                .unwrap();
        assert_eq!(
            authorization.replacement_run_id,
            AGENT_TOKEN_SPRINT_ON_REPLACEMENT_RUN_ID
        );
        assert_eq!(
            authorization.policy_revision,
            AGENT_TOKEN_SPRINT_ON_REPLACEMENT_POLICY_REVISION
        );
        assert!(authorization
            .pair_admission_policy
            .contains("host_shutdown_whole_pair_recovery"));

        let mut selection = authorized_selection();
        selection.campaign_id = manifest.campaign_id.clone();
        selection.policy_revision = authorization.policy_revision.to_string();
        selection.source_run_id = authorization.source_run_id.to_string();
        selection.replacement_run_id = authorization.replacement_run_id.to_string();
        selection.replacement_run_summary = format!(
            "statistical-replacements/replacement-0001/runs/{}/run-summary.json",
            authorization.replacement_run_id
        );
        validate_selection_identity(&selection, &manifest).unwrap();

        assert!(
            statistical_replacement_authorization(&manifest, AGENT_TOKEN_REPLACED_RUN_ID,).is_err()
        );
        let mut wrong_policy = selection;
        wrong_policy.policy_revision = AGENT_TOKEN_REPLACEMENT_POLICY_REVISION.to_string();
        assert!(validate_selection_identity(&wrong_policy, &manifest).is_err());
    }

    #[test]
    fn prospective_replacement_authorization_is_manifest_declared_and_mode_symmetric() {
        let mut manifest = authorized_manifest();
        manifest.campaign_id = "fable-complete".to_string();
        manifest.protocol_revision = crate::AGENT_TOKEN_PROTOCOL_REVISION.to_string();
        manifest.functional_replacement_policy =
            crate::AgentTokenFunctionalReplacementPolicy::FirstValidUnacceptedLaneOnce;

        for mode in ["git", "ait"] {
            let source_run_id = format!("fable-complete-b001-gd-01-{mode}");
            let authorization =
                statistical_replacement_authorization(&manifest, &source_run_id).unwrap();
            assert_eq!(authorization.source_run_id, source_run_id);
            assert_eq!(
                authorization.replacement_run_id,
                format!("{source_run_id}-replacement-01")
            );
            assert_eq!(
                authorization.reason,
                AGENT_TOKEN_PROSPECTIVE_REPLACEMENT_REASON
            );
        }

        manifest.functional_replacement_policy = crate::AgentTokenFunctionalReplacementPolicy::None;
        assert!(
            statistical_replacement_authorization(&manifest, "fable-complete-b001-gd-01-git")
                .is_err()
        );
    }

    #[test]
    fn replacement_paths_must_remain_portable_and_relative() {
        assert_eq!(
            portable_relative_path("statistical-replacements/replacement-0001/run.json").unwrap(),
            PathBuf::from("statistical-replacements/replacement-0001/run.json")
        );
        for rejected in ["", "/absolute/run.json", "../escaped.json", "a/../run.json"] {
            assert!(portable_relative_path(rejected).is_err(), "{rejected}");
        }
    }

    #[test]
    fn replacement_prompt_allows_only_exact_run_identity_substitution() {
        let source_id = "campaign-b006-gd-05-ait";
        let replacement_id = "campaign-b006-gd-05-ait-replacement-01";
        let source = format!(
            "plan-ref: {source_id}/root\nref: {source_id}/implement\nstart #{source_id}/implement\n"
        );
        let replacement = source.replace(source_id, replacement_id);
        require_exact_run_identity_substitution(
            source.as_bytes(),
            replacement.as_bytes(),
            source_id,
            replacement_id,
            "test prompt",
        )
        .unwrap();

        let drifted = replacement.replace("start", "start with extra coaching");
        assert!(require_exact_run_identity_substitution(
            source.as_bytes(),
            drifted.as_bytes(),
            source_id,
            replacement_id,
            "test prompt",
        )
        .is_err());
        assert!(require_exact_run_identity_substitution(
            b"unchanged sprint-off prompt",
            b"unchanged sprint-off prompt",
            source_id,
            replacement_id,
            "test prompt",
        )
        .is_ok());
    }
}
