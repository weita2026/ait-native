use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    build_agent_token_report, load_agent_token_run_summaries, sha256_digest,
    AgentTokenCampaignManifest, AgentTokenEnvironment, AgentTokenExecutorPreflightReport,
    AgentTokenMode, AgentTokenReport, AgentTokenRunManifest, AgentTokenRunSummary,
    AgentTokenSchedule, AgentTokenStatisticalReplacementRecord,
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
}

pub fn load_agent_token_campaign_statistical_view(
    manifest: &AgentTokenCampaignManifest,
    source_schedule: &AgentTokenSchedule,
    campaign_dir: &Path,
) -> Result<AgentTokenCampaignStatisticalView, String> {
    let source_runs = load_agent_token_run_summaries(campaign_dir)?;
    let selection_path = campaign_dir.join(AGENT_TOKEN_REPLACEMENT_SELECTION_FILE);
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
        });
    }

    require_regular_file(&selection_path, "statistical replacement selection")?;
    let selection = read_json::<AgentTokenStatisticalReplacementSelection>(
        &selection_path,
        "statistical replacement selection",
    )?;
    validate_selection_identity(&selection, manifest)?;

    let source_index = source_runs
        .iter()
        .position(|run| run.run_id == selection.source_run_id)
        .ok_or_else(|| {
            format!(
                "Statistical replacement source run {} is absent from the campaign",
                selection.source_run_id
            )
        })?;
    let source_run = source_runs[source_index].clone();
    if !source_run.valid_attempt || source_run.accepted_equivalent {
        return Err(format!(
            "Statistical replacement source run {} must be valid and unaccepted",
            source_run.run_id
        ));
    }
    let source_summary_path = campaign_dir
        .join("runs")
        .join(&source_run.run_id)
        .join("run-summary.json");
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

    let mut effective_schedule = source_schedule.clone();
    let schedule_entry = effective_schedule
        .entries
        .iter_mut()
        .find(|entry| entry.run_id == source_run.run_id)
        .ok_or_else(|| {
            format!(
                "Statistical replacement source run {} is absent from the frozen schedule",
                source_run.run_id
            )
        })?;
    schedule_entry.run_id = replacement_run.run_id.clone();

    let mut effective_runs = source_runs.clone();
    effective_runs[source_index] = replacement_run.clone();
    let order = effective_schedule
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.run_id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    effective_runs.sort_by_key(|run| order.get(run.run_id.as_str()).copied());

    let source_report = build_agent_token_report(manifest, source_schedule, &source_runs)?;
    let mut report = build_agent_token_report(manifest, &effective_schedule, &effective_runs)?;
    report.source_protocol_claim_eligible = source_report.source_protocol_claim_eligible;
    report.source_protocol_blockers = source_report.blockers.clone();
    report.current_policy_revision = AGENT_TOKEN_REPLACEMENT_POLICY_REVISION.to_string();
    report.current_policy_evaluation_mode = "owner_authorized_statistical_replacement".to_string();
    report.current_policy_criteria_met = report.blockers.is_empty() && report.claim_eligible;
    report.current_policy_blockers = report.blockers.clone();
    report.claim_eligible = report.current_policy_criteria_met;
    report.executed_evidence_run_count = source_runs.len().saturating_add(1);
    report.statistically_excluded_run_count = 1;
    report.replacement_policy_revision = Some(AGENT_TOKEN_REPLACEMENT_POLICY_REVISION.to_string());
    report.pair_admission_policy = AGENT_TOKEN_REPLACEMENT_PAIR_ADMISSION_POLICY.to_string();
    report.statistical_replacements = vec![AgentTokenStatisticalReplacementRecord {
        source_run_id: selection.source_run_id.clone(),
        replacement_run_id: selection.replacement_run_id.clone(),
        source_run_summary_sha256: selection.source_run_summary_sha256.clone(),
        replacement_run_summary_sha256: selection.replacement_run_summary_sha256.clone(),
        replacement_runner_sha256: selection.replacement_runner_sha256.clone(),
        reason: selection.reason.clone(),
    }];
    report.limitations.push(
        "The disclosed valid GD-05 AIT functional failure remains in append-only evidence but is excluded from effective statistics under the repository-owner-authorized replacement policy."
            .to_string(),
    );

    let mut effective_run_summary_paths = source_runs
        .iter()
        .filter(|run| run.run_id != source_run.run_id)
        .map(|run| {
            (
                run.run_id.clone(),
                campaign_dir
                    .join("runs")
                    .join(&run.run_id)
                    .join("run-summary.json"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    effective_run_summary_paths.insert(replacement_run.run_id.clone(), replacement_summary_path);
    let excluded_run_summary_paths =
        BTreeMap::from([(source_run.run_id.clone(), source_summary_path)]);

    Ok(AgentTokenCampaignStatisticalView {
        report,
        effective_schedule,
        effective_runs,
        excluded_runs: vec![source_run],
        effective_run_summary_paths,
        excluded_run_summary_paths,
        selection: Some(selection),
    })
}

pub(crate) fn validate_selection_identity(
    selection: &AgentTokenStatisticalReplacementSelection,
    manifest: &AgentTokenCampaignManifest,
) -> Result<(), String> {
    if selection.contract != AGENT_TOKEN_REPLACEMENT_SELECTION_CONTRACT
        || selection.campaign_id != AGENT_TOKEN_REPLACEMENT_CAMPAIGN_ID
        || selection.campaign_id != manifest.campaign_id
        || selection.policy_revision != AGENT_TOKEN_REPLACEMENT_POLICY_REVISION
        || selection.source_run_id != AGENT_TOKEN_REPLACED_RUN_ID
        || selection.replacement_run_id != AGENT_TOKEN_REPLACEMENT_RUN_ID
        || selection.reason != AGENT_TOKEN_REPLACEMENT_REASON
        || selection.selected_at.trim().is_empty()
    {
        return Err("Statistical replacement selection identity differs from the narrow owner-authorized contract".to_string());
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
        || replacement.mode != AgentTokenMode::AitLinearSingleSession
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
    for identical in [
        "campaign-manifest.json",
        "fixture-manifest.json",
        "prompt.txt",
    ] {
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
    if source_manifest.fixture_revision != replacement_manifest.fixture_revision
        || source_manifest.fixture_content_digest != replacement_manifest.fixture_content_digest
        || source_manifest.shared_task_prompt_digest
            != replacement_manifest.shared_task_prompt_digest
        || source_manifest.measured_prompt_digest != replacement_manifest.measured_prompt_digest
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
        AgentTokenAccountingProfile, AgentTokenCampaignScope, AgentTokenModelPin, AgentTokenRuntime,
    };

    fn authorized_manifest() -> AgentTokenCampaignManifest {
        AgentTokenCampaignManifest {
            contract: crate::AGENT_TOKEN_CAMPAIGN_CONTRACT.to_string(),
            campaign_id: AGENT_TOKEN_REPLACEMENT_CAMPAIGN_ID.to_string(),
            protocol_revision: crate::AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION
                .to_string(),
            campaign_scope: AgentTokenCampaignScope::Complete,
            accounting_profile: AgentTokenAccountingProfile::SteadyStateTaskCost,
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
    fn replacement_paths_must_remain_portable_and_relative() {
        assert_eq!(
            portable_relative_path("statistical-replacements/replacement-0001/run.json").unwrap(),
            PathBuf::from("statistical-replacements/replacement-0001/run.json")
        );
        for rejected in ["", "/absolute/run.json", "../escaped.json", "a/../run.json"] {
            assert!(portable_relative_path(rejected).is_err(), "{rejected}");
        }
    }
}
