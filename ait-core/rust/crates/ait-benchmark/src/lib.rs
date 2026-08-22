mod agent_token;
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
    build_agent_token_report, build_agent_token_schedule, compare_agent_token_reports,
    extract_agent_token_secondary_metrics, extract_and_validate_codex_transcript,
    import_codex_usage, load_agent_token_campaign, load_agent_token_report,
    load_agent_token_run_summaries, load_agent_token_schedule, render_agent_token_report_markdown,
    validate_agent_token_campaign, write_json_new, write_text_new, AgentTokenAccountingProfile,
    AgentTokenBrowserReport, AgentTokenCampaignManifest, AgentTokenCampaignScope,
    AgentTokenCommandTranscript, AgentTokenCrossCampaignDelta, AgentTokenCrossCampaignReport,
    AgentTokenEnvironment, AgentTokenGroupReport, AgentTokenMode, AgentTokenModeComparison,
    AgentTokenModelPin, AgentTokenReport, AgentTokenRunSummary, AgentTokenRuntime,
    AgentTokenSchedule, AgentTokenScheduleEntry, AgentTokenSecondaryMetrics,
    NormalizedAgentTokenUsage, AGENT_TOKEN_BROWSER_REPORT_CONTRACT, AGENT_TOKEN_CAMPAIGN_CONTRACT,
    AGENT_TOKEN_ENVIRONMENT_CONTRACT, AGENT_TOKEN_PROTOCOL_REVISION, AGENT_TOKEN_REPORT_CONTRACT,
    AGENT_TOKEN_RUN_SUMMARY_CONTRACT, AGENT_TOKEN_SCHEDULE_CONTRACT,
    AGENT_TOKEN_TRANSCRIPT_CONTRACT, AGENT_TOKEN_USAGE_CONTRACT,
};
pub use agent_token_runner::{
    run_agent_token_campaign, validate_agent_token_campaign_evidence, AgentTokenCampaignExecution,
    AgentTokenRunIndex, AgentTokenRunIndexEntry, AgentTokenRunManifest,
    AgentTokenWorkflowVerification, AGENT_TOKEN_CAMPAIGN_EXECUTION_CONTRACT,
    AGENT_TOKEN_RUN_INDEX_CONTRACT, AGENT_TOKEN_RUN_MANIFEST_CONTRACT,
    AGENT_TOKEN_WORKFLOW_VERIFICATION_CONTRACT,
};
pub use comparison::{
    compare_reports, load_benchmark_report, load_budget_manifest, render_comparison_markdown,
    write_comparison_report, BenchmarkBudgetManifest, BenchmarkComparisonReport, BudgetResult,
    BudgetRule,
};
