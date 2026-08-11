mod comparison;
mod fixture;
mod model;
mod protocol;
mod report;
mod runner;
mod statistics;

pub use fixture::{
    create_synthetic_fixture, digest_workspace, profile_workspace, SyntheticFixtureReceipt,
};
pub use model::*;
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
pub use comparison::{
    compare_reports, load_benchmark_report, load_budget_manifest, render_comparison_markdown,
    write_comparison_report, BenchmarkBudgetManifest, BenchmarkComparisonReport, BudgetResult,
    BudgetRule,
};
