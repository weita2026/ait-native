mod agent_token;
mod agent_token_host_shutdown_recovery;
mod agent_token_infrastructure_recovery;
mod agent_token_publication;
mod agent_token_replacement;
mod agent_token_runner;
mod comparison;
mod fixture;
mod game_fixture;
mod model;
mod portable;
mod protocol;
mod report;
mod runner;
mod statistics;

pub use fixture::{
    create_synthetic_fixture, digest_workspace, profile_workspace, SyntheticFixtureReceipt,
};
pub use game_fixture::{
    load_game_fixture_manifest, materialize_game_fixture, GameFixtureManifest, GameFixtureReceipt,
    GameSourceReplacement, GameSourceTransform, GameWorkloadDeclaration,
    GAME_FIXTURE_MANIFEST_CONTRACT, GAME_FIXTURE_RECEIPT_CONTRACT, GAME_FIXTURE_TRANSFORM_CONTRACT,
};
pub use model::*;
pub use portable::{
    encode_manifest, normalize_manifest, resolve_manifest_bindings, sha256_digest,
    validate_portable_manifest, NormalizationReport, NormalizedManifest, PortabilityReport,
    RuntimeBindings, NORMALIZATION_CONTRACT, PORTABILITY_CONTRACT,
};
pub use protocol::{load_manifest, validate_manifest, ValidationReport};
pub use report::{build_report, render_markdown, write_report};
pub use runner::{run_benchmark, RunOptions, RunSummary};
pub use statistics::{summarize_samples, DistributionSummary};

pub const MANIFEST_CONTRACT: &str = "ait-vcs-benchmark-manifest/v1";
pub const RAW_CONTRACT: &str = "ait-vcs-benchmark-raw/v1";
pub const REPORT_CONTRACT: &str = "ait-vcs-benchmark-report/v1";
pub const FIXTURE_CONTRACT: &str = "ait-vcs-benchmark-fixture/v1";
pub const BUDGET_CONTRACT: &str = "ait-vcs-benchmark-budget/v1";
pub const COMPARISON_CONTRACT: &str = "ait-vcs-benchmark-comparison/v1";
pub const PROTOCOL_V1_JSON: &str = include_str!("../protocol/v1.json");
pub const AGENT_TOKEN_PROTOCOL_V1_JSON: &str = include_str!("../protocol/agent-token-v1.json");
pub use agent_token::{
    build_agent_token_report, build_agent_token_run_adjudication, build_agent_token_schedule,
    compare_agent_token_reports, extract_agent_token_claude_secondary_metrics,
    extract_agent_token_secondary_metrics, extract_and_validate_claude_transcript,
    extract_and_validate_codex_transcript, import_claude_usage, import_codex_usage,
    load_agent_token_campaign, load_agent_token_campaign_for_evidence,
    load_agent_token_raw_run_summaries, load_agent_token_report, load_agent_token_run_summaries,
    load_agent_token_schedule, render_agent_token_report_markdown, validate_agent_token_campaign,
    write_json_new, write_text_new, AgentTokenAccountingProfile, AgentTokenAitSprintMode,
    AgentTokenBrowserReport, AgentTokenCampaignManifest, AgentTokenCampaignScope,
    AgentTokenCommandTranscript, AgentTokenCrossCampaignDelta, AgentTokenCrossCampaignReport,
    AgentTokenEnvironment, AgentTokenFunctionalReplacementPolicy, AgentTokenGitWorktreeMode,
    AgentTokenGroupReport, AgentTokenHostShutdownPairRecoveryRecord,
    AgentTokenInfrastructurePairRecoveryRecord, AgentTokenMode, AgentTokenModeComparison,
    AgentTokenModelPin, AgentTokenRecoveredSpawnAdjudicationRecord, AgentTokenReport,
    AgentTokenRunAdjudication, AgentTokenRunSummary, AgentTokenRuntime, AgentTokenSchedule,
    AgentTokenScheduleEntry, AgentTokenSecondaryMetrics, AgentTokenServedModelReport,
    AgentTokenServedModelUsage, AgentTokenStatisticalReplacementRecord, NormalizedAgentTokenUsage,
    AGENT_TOKEN_200_SESSION_PREDECESSOR_PROTOCOL_REVISION, AGENT_TOKEN_ADJUDICATOR_REVISION,
    AGENT_TOKEN_BROWSER_REPORT_CONTRACT, AGENT_TOKEN_CAMPAIGN_CONTRACT,
    AGENT_TOKEN_COMPLETE_PREDECESSOR_PROTOCOL_REVISIONS, AGENT_TOKEN_ENVIRONMENT_CONTRACT,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_LEGACY_ADJUDICATOR_REVISION, AGENT_TOKEN_LEGACY_RESUMABLE_PROTOCOL_REVISION,
    AGENT_TOKEN_MANAGED_WORKTREE_PREFLIGHT_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_MODEL_ADMISSION_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_MODEL_PURITY_PREDECESSOR_PROTOCOL_REVISION, AGENT_TOKEN_PAIR_ADMISSION_POLICY,
    AGENT_TOKEN_PRE_REPLACEMENT_PROTOCOL_REVISION,
    AGENT_TOKEN_PROMPTED_INSPECTION_PREDECESSOR_PROTOCOL_REVISION, AGENT_TOKEN_PROTOCOL_REVISION,
    AGENT_TOKEN_RECOVERED_SPAWN_CAMPAIGN_ID, AGENT_TOKEN_RECOVERED_SPAWN_PAIR_ADMISSION_POLICY,
    AGENT_TOKEN_RECOVERED_SPAWN_PAIR_START_INDEX, AGENT_TOKEN_RECOVERED_SPAWN_POLICY_REVISION,
    AGENT_TOKEN_RECOVERED_SPAWN_REASON, AGENT_TOKEN_RECOVERED_SPAWN_RUN_ID,
    AGENT_TOKEN_RECOVERED_SPAWN_SOURCE_SUMMARY_SHA256, AGENT_TOKEN_REPORT_CONTRACT,
    AGENT_TOKEN_RUN_ADJUDICATION_CONTRACT, AGENT_TOKEN_RUN_SUMMARY_CONTRACT,
    AGENT_TOKEN_SCHEDULE_CONTRACT, AGENT_TOKEN_SPRINT_OFF_PREDECESSOR_PROTOCOL_REVISION,
    AGENT_TOKEN_SPRINT_ON_COMPLETE_PREDECESSOR_PROTOCOL_REVISION, AGENT_TOKEN_TRANSCRIPT_CONTRACT,
    AGENT_TOKEN_USAGE_CONTRACT, AGENT_TOKEN_VALID_OUTCOME_RESUMABLE_PROTOCOL_REVISION,
};
pub use agent_token_host_shutdown_recovery::{
    capture_host_shutdown_observation, classify_host_shutdown_interruption,
    host_shutdown_replacement_run_id, load_agent_token_host_shutdown_recovery_view,
    validate_host_shutdown_observation, AgentTokenHostShutdownInterruption,
    AgentTokenHostShutdownObservation, AgentTokenHostShutdownPairRecoverySelection,
    AgentTokenHostShutdownRecoveryView, AgentTokenInterruptedArtifact,
    AGENT_TOKEN_HOST_REBOOT_AT_UNIX_S, AGENT_TOKEN_HOST_SHUTDOWN_AT_UNIX_S,
    AGENT_TOKEN_HOST_SHUTDOWN_CAMPAIGN_ID, AGENT_TOKEN_HOST_SHUTDOWN_INTERRUPTED_RUN_ID,
    AGENT_TOKEN_HOST_SHUTDOWN_OBSERVATION_CONTRACT, AGENT_TOKEN_HOST_SHUTDOWN_PAIR_START_INDEX,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_CONTRACT,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_PAIR_ADMISSION_POLICY,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_POLICY_REVISION, AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_REASON,
    AGENT_TOKEN_HOST_SHUTDOWN_RECOVERY_SELECTION_FILE,
};
pub use agent_token_infrastructure_recovery::{
    load_agent_token_infrastructure_recovery_view, recognized_infrastructure_failure,
    replacement_run_id, AgentTokenInfrastructurePairRecoverySelection,
    AgentTokenInfrastructureRecoveryArtifact, AgentTokenInfrastructureRecoveryView,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_CONTRACT,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_PAIR_ADMISSION_POLICY,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_POLICY_REVISION,
    AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_REASON, AGENT_TOKEN_INFRASTRUCTURE_RECOVERY_SELECTION_FILE,
};
pub use agent_token_publication::{
    write_agent_token_publication_bundle, AgentTokenPublicFailure, AgentTokenPublicRun,
    AgentTokenPublicRunIndex, AgentTokenPublicationInput, AgentTokenPublicationReceipt,
    AgentTokenPublicationResult, AGENT_TOKEN_PUBLICATION_CONTRACT,
    AGENT_TOKEN_PUBLIC_RUN_INDEX_CONTRACT,
};
pub use agent_token_replacement::{
    load_agent_token_campaign_statistical_view, AgentTokenCampaignStatisticalView,
    AgentTokenStatisticalReplacementSelection, AGENT_TOKEN_REPLACED_RUN_ID,
    AGENT_TOKEN_REPLACEMENT_CAMPAIGN_ID, AGENT_TOKEN_REPLACEMENT_PAIR_ADMISSION_POLICY,
    AGENT_TOKEN_REPLACEMENT_POLICY_REVISION, AGENT_TOKEN_REPLACEMENT_REASON,
    AGENT_TOKEN_REPLACEMENT_RUN_ID, AGENT_TOKEN_REPLACEMENT_SELECTION_CONTRACT,
    AGENT_TOKEN_REPLACEMENT_SELECTION_FILE,
};
pub use agent_token_runner::{
    enrich_agent_token_report_model_composition, resume_agent_token_campaign,
    run_agent_token_campaign, run_agent_token_statistical_replacement,
    validate_agent_token_campaign_evidence, AgentTokenCampaignExecution, AgentTokenCampaignResume,
    AgentTokenCodexPermissionProfile, AgentTokenExecutorPreflightEnvironment,
    AgentTokenExecutorPreflightReport, AgentTokenExecutorPreflightUsage,
    AgentTokenGitStartStateProof, AgentTokenGitWorktreePermissionPreflightReport,
    AgentTokenManagedWorktreeLifecycle, AgentTokenModelRequestState,
    AgentTokenReplacementExecution, AgentTokenRunIndex, AgentTokenRunIndexEntry,
    AgentTokenRunManifest, AgentTokenWorkflowVerification, AGENT_TOKEN_CAMPAIGN_EXECUTION_CONTRACT,
    AGENT_TOKEN_CAMPAIGN_RESUME_CONTRACT, AGENT_TOKEN_CODEX_PERMISSION_PROFILE_CONTRACT,
    AGENT_TOKEN_EXECUTOR_PREFLIGHT_COMMAND_COUNT, AGENT_TOKEN_EXECUTOR_PREFLIGHT_CONTRACT,
    AGENT_TOKEN_EXECUTOR_PREFLIGHT_ENVIRONMENT_CONTRACT,
    AGENT_TOKEN_EXECUTOR_PREFLIGHT_USAGE_CONTRACT, AGENT_TOKEN_GIT_START_STATE_PROOF_CONTRACT,
    AGENT_TOKEN_GIT_WORKTREE_PERMISSION_PREFLIGHT_CONTRACT,
    AGENT_TOKEN_MANAGED_WORKTREE_LIFECYCLE_CONTRACT, AGENT_TOKEN_MODEL_REQUEST_STATE_CONTRACT,
    AGENT_TOKEN_REPLACEMENT_EXECUTION_CONTRACT, AGENT_TOKEN_RUN_INDEX_CONTRACT,
    AGENT_TOKEN_RUN_MANIFEST_CONTRACT, AGENT_TOKEN_WORKFLOW_VERIFICATION_CONTRACT,
};
pub use comparison::{
    compare_reports, load_benchmark_report, load_budget_manifest, render_comparison_markdown,
    write_comparison_report, BenchmarkBudgetManifest, BenchmarkComparisonReport, BudgetResult,
    BudgetRule,
};
