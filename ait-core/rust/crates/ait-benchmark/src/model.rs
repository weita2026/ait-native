use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkManifest {
    pub contract: String,
    pub benchmark_id: String,
    pub protocol_revision: String,
    #[serde(default)]
    pub campaign_scope: CampaignScope,
    pub seed: u64,
    pub sampling: SamplingPolicy,
    pub environment: EnvironmentPin,
    pub fixtures: Vec<FixtureDeclaration>,
    pub cells: Vec<CellSpec>,
    #[serde(default = "default_bootstrap_resamples")]
    pub bootstrap_resamples: usize,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CampaignScope {
    #[default]
    FullMatrix,
    FocusedSlice,
}

impl CampaignScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FullMatrix => "full_matrix",
            Self::FocusedSlice => "focused_slice",
        }
    }

    pub fn claim_eligible(&self) -> bool {
        matches!(self, Self::FullMatrix)
    }
}

fn default_bootstrap_resamples() -> usize {
    2_000
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SamplingPolicy {
    pub warmup_iterations: usize,
    pub measured_local_iterations: usize,
    pub measured_cold_iterations: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPin {
    pub captured_at: String,
    pub os: String,
    pub architecture: String,
    pub filesystem: String,
    pub storage_medium: String,
    pub cpu: String,
    pub memory_bytes: u64,
    pub rust_version: String,
    pub git_version: String,
    pub ait_version: String,
    pub repository_snapshot: String,
    pub server_revision: String,
    pub network_profile: String,
    pub cache_drop_method: String,
    #[serde(default)]
    pub command_options: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureScale {
    Small,
    Medium,
    Large,
}

impl FixtureScale {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureDeclaration {
    pub fixture_id: String,
    pub revision: String,
    pub scale: FixtureScale,
    pub kind: String,
    pub source: String,
    pub redistribution: String,
    pub content_digest: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub history_nodes: u64,
    pub features: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SampleClass {
    Local,
    ProcessNetwork,
}

impl SampleClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::ProcessNetwork => "process_network",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Temperature {
    Cold,
    Warm,
}

impl Temperature {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Warm => "warm",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CellSpec {
    pub cell_id: String,
    pub fixture_id: String,
    pub operation: String,
    pub temperature: Temperature,
    pub sample_class: SampleClass,
    pub subjects: Vec<SubjectSpec>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectSpec {
    pub subject_id: String,
    pub role: String,
    pub workspace_root: PathBuf,
    #[serde(default = "default_metadata_excludes")]
    pub metadata_excludes: Vec<String>,
    pub command: CommandSpec,
    #[serde(default)]
    pub reset_commands: Vec<CommandSpec>,
    #[serde(default)]
    pub prepare_commands: Vec<CommandSpec>,
    #[serde(default)]
    pub cleanup_commands: Vec<CommandSpec>,
    pub history_node_probe: CommandSpec,
    pub outcome_probe: CommandSpec,
    pub metrics_json_path: Option<PathBuf>,
}

fn default_metadata_excludes() -> Vec<String> {
    vec![".ait".to_string(), ".git".to_string()]
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "default_expected_exit_codes")]
    pub expected_exit_codes: Vec<i32>,
}

fn default_expected_exit_codes() -> Vec<i32> {
    vec![0]
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "record_type", rename_all = "snake_case")]
pub enum RawRecord {
    Header(RawHeader),
    Sample(SampleRecord),
    Footer(RawFooter),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RawHeader {
    pub contract: String,
    pub benchmark_id: String,
    pub protocol_revision: String,
    pub manifest_digest: String,
    pub started_at: String,
    pub evidence_class: String,
    pub protocol_conformant: bool,
    #[serde(default = "default_raw_campaign_scope")]
    pub campaign_scope: String,
    pub seed: u64,
    pub environment: EnvironmentPin,
}

fn default_raw_campaign_scope() -> String {
    CampaignScope::FullMatrix.as_str().to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SampleRecord {
    pub contract: String,
    pub benchmark_id: String,
    pub protocol_revision: String,
    pub cell_id: String,
    pub fixture_id: String,
    pub fixture_scale: String,
    pub fixture_content_digest: String,
    pub operation: String,
    pub temperature: String,
    pub sample_class: String,
    pub subject_id: String,
    pub subject_role: String,
    pub block_index: usize,
    pub randomized_order: usize,
    pub warmup: bool,
    pub started_at: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub wall_time_ns: u64,
    pub cpu_user_ns: Option<u64>,
    pub cpu_system_ns: Option<u64>,
    pub peak_rss_bytes: Option<u64>,
    pub io_read_bytes: Option<u64>,
    pub io_write_bytes: Option<u64>,
    pub transferred_bytes: Option<u64>,
    pub server_latency_ns: Option<u64>,
    pub server_health_ok: Option<bool>,
    pub outcome_digest: Option<String>,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RawFooter {
    pub contract: String,
    pub benchmark_id: String,
    pub finished_at: String,
    pub sample_count: usize,
    pub measured_sample_count: usize,
    pub failure_count: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SidecarMetrics {
    pub transferred_bytes: Option<u64>,
    pub server_latency_ns: Option<u64>,
    pub server_health_ok: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SyntheticFixtureRecipe {
    pub contract: String,
    pub fixture_id: String,
    pub revision: String,
    pub scale: FixtureScale,
    pub seed: u64,
    pub file_count: u64,
    pub total_bytes: u64,
    pub history_nodes: u64,
    pub max_depth: usize,
    pub binary_percent: u8,
    pub ignored_percent: u8,
    pub features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BenchmarkReport {
    pub contract: String,
    pub benchmark_id: String,
    pub protocol_revision: String,
    pub generated_at: String,
    pub manifest_digest: String,
    pub raw_jsonl_path: String,
    pub environment: EnvironmentPin,
    pub evidence_class: String,
    pub protocol_conformant: bool,
    pub campaign_scope: String,
    pub claim_eligible: bool,
    pub total_failure_count: usize,
    pub cells: Vec<CellReport>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CellReport {
    pub cell_id: String,
    pub fixture_id: String,
    pub fixture_scale: String,
    pub operation: String,
    pub temperature: String,
    pub sample_class: String,
    pub subjects: Vec<SubjectReport>,
    pub ait_vs_git: Option<CellComparison>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CellComparison {
    pub candidate_subject_id: String,
    pub baseline_subject_id: String,
    pub p50_wall_time_ratio: Option<f64>,
    pub p95_wall_time_ratio: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SubjectReport {
    pub subject_id: String,
    pub role: String,
    pub measured_sample_count: usize,
    pub failure_count: usize,
    pub wall_time_ns: Option<crate::statistics::DistributionSummary>,
    pub cpu_time_ns: Option<crate::statistics::DistributionSummary>,
    pub peak_rss_bytes: Option<crate::statistics::DistributionSummary>,
    pub io_read_bytes: Option<crate::statistics::DistributionSummary>,
    pub io_write_bytes: Option<crate::statistics::DistributionSummary>,
    pub transferred_bytes: Option<crate::statistics::DistributionSummary>,
    pub server_latency_ns: Option<crate::statistics::DistributionSummary>,
    pub server_health_failure_count: usize,
}
