use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::model::{BenchmarkReport, CellReport, EnvironmentPin};
use crate::{BUDGET_CONTRACT, COMPARISON_CONTRACT, REPORT_CONTRACT};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkBudgetManifest {
    pub contract: String,
    pub budget_id: String,
    pub revision: String,
    pub rules: Vec<BudgetRule>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetRule {
    pub rule_id: String,
    pub operation: String,
    #[serde(default)]
    pub fixture_scale: Option<String>,
    #[serde(default)]
    pub temperature: Option<String>,
    #[serde(default)]
    pub sample_class: Option<String>,
    #[serde(default)]
    pub min_p50_reduction_percent: Option<f64>,
    #[serde(default)]
    pub max_p95_regression_percent: Option<f64>,
    #[serde(default)]
    pub max_candidate_ait_git_p50_ratio: Option<f64>,
    #[serde(default)]
    pub max_candidate_ait_git_p95_ratio: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BenchmarkComparisonReport {
    pub contract: String,
    pub generated_at: String,
    pub budget_id: String,
    pub budget_revision: String,
    pub baseline_benchmark_id: String,
    pub candidate_benchmark_id: String,
    pub baseline_claim_eligible: bool,
    pub candidate_claim_eligible: bool,
    pub environment_comparable: bool,
    pub environment_differences: Vec<String>,
    pub matrix_complete: bool,
    pub budget_passed: bool,
    pub promotion_ready: bool,
    pub cells: Vec<BeforeAfterCell>,
    pub budget_results: Vec<BudgetResult>,
    pub blockers: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BeforeAfterCell {
    pub cell_key: String,
    pub fixture_scale: String,
    pub operation: String,
    pub temperature: String,
    pub sample_class: String,
    pub baseline_cell_id: Option<String>,
    pub candidate_cell_id: Option<String>,
    pub baseline_ait_p50_ns: Option<f64>,
    pub baseline_ait_p95_ns: Option<f64>,
    pub candidate_ait_p50_ns: Option<f64>,
    pub candidate_ait_p95_ns: Option<f64>,
    pub p50_reduction_percent: Option<f64>,
    pub p95_change_percent: Option<f64>,
    pub baseline_ait_git_p50_ratio: Option<f64>,
    pub baseline_ait_git_p95_ratio: Option<f64>,
    pub candidate_ait_git_p50_ratio: Option<f64>,
    pub candidate_ait_git_p95_ratio: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BudgetResult {
    pub rule_id: String,
    pub cell_key: Option<String>,
    pub status: String,
    pub reasons: Vec<String>,
}

pub fn load_benchmark_report(path: &Path) -> Result<BenchmarkReport, String> {
    read_json(path, "benchmark report")
}

pub fn load_budget_manifest(path: &Path) -> Result<BenchmarkBudgetManifest, String> {
    let budget = read_json::<BenchmarkBudgetManifest>(path, "benchmark budget")?;
    validate_budget(&budget)?;
    Ok(budget)
}

pub fn compare_reports(
    baseline: &BenchmarkReport,
    candidate: &BenchmarkReport,
    budget: &BenchmarkBudgetManifest,
) -> Result<BenchmarkComparisonReport, String> {
    if baseline.contract != REPORT_CONTRACT || candidate.contract != REPORT_CONTRACT {
        return Err(format!(
            "Both inputs must use report contract {REPORT_CONTRACT}"
        ));
    }
    validate_budget(budget)?;

    let environment_differences =
        environment_differences(&baseline.environment, &candidate.environment);
    let environment_comparable = environment_differences.is_empty();
    let baseline_cells = index_cells(&baseline.cells, "baseline")?;
    let candidate_cells = index_cells(&candidate.cells, "candidate")?;
    let keys = baseline_cells
        .keys()
        .chain(candidate_cells.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let matrix_complete = baseline_cells.keys().eq(candidate_cells.keys());
    let cells = keys
        .iter()
        .map(|key| compare_cell(key, baseline_cells.get(key), candidate_cells.get(key)))
        .collect::<Vec<_>>();

    let mut budget_results = Vec::new();
    for rule in &budget.rules {
        let matches = cells
            .iter()
            .filter(|cell| rule_matches(rule, cell))
            .collect::<Vec<_>>();
        if matches.is_empty() {
            budget_results.push(BudgetResult {
                rule_id: rule.rule_id.clone(),
                cell_key: None,
                status: "missing".to_string(),
                reasons: vec!["no before/after cell matches this budget rule".to_string()],
            });
            continue;
        }
        for cell in matches {
            budget_results.push(evaluate_rule(rule, cell));
        }
    }
    let budget_passed = budget_results
        .iter()
        .all(|result| result.status == "passed");

    let mut blockers = Vec::new();
    if !baseline.claim_eligible {
        blockers.push(format!(
            "baseline {} is not claim eligible (scope {}, evidence class {})",
            baseline.benchmark_id, baseline.campaign_scope, baseline.evidence_class
        ));
    }
    if !candidate.claim_eligible {
        blockers.push(format!(
            "candidate {} is not claim eligible (scope {}, evidence class {})",
            candidate.benchmark_id, candidate.campaign_scope, candidate.evidence_class
        ));
    }
    if !environment_comparable {
        blockers.push("baseline and candidate environments are not comparable".to_string());
    }
    if !matrix_complete {
        blockers.push("baseline and candidate report cell sets differ".to_string());
    }
    if !budget_passed {
        blockers.push("one or more ratified budget rules failed or lack evidence".to_string());
    }
    let promotion_ready = blockers.is_empty();
    let mut limitations = budget.limitations.clone();
    limitations.extend(baseline.limitations.iter().cloned());
    limitations.extend(candidate.limitations.iter().cloned());
    limitations.sort();
    limitations.dedup();

    Ok(BenchmarkComparisonReport {
        contract: COMPARISON_CONTRACT.to_string(),
        generated_at: Utc::now().to_rfc3339(),
        budget_id: budget.budget_id.clone(),
        budget_revision: budget.revision.clone(),
        baseline_benchmark_id: baseline.benchmark_id.clone(),
        candidate_benchmark_id: candidate.benchmark_id.clone(),
        baseline_claim_eligible: baseline.claim_eligible,
        candidate_claim_eligible: candidate.claim_eligible,
        environment_comparable,
        environment_differences,
        matrix_complete,
        budget_passed,
        promotion_ready,
        cells,
        budget_results,
        blockers,
        limitations,
    })
}

pub fn write_comparison_report(
    report: &BenchmarkComparisonReport,
    json_path: &Path,
    markdown_path: Option<&Path>,
) -> Result<(), String> {
    ensure_parent(json_path)?;
    let file = File::create(json_path)
        .map_err(|error| format!("Failed to create {}: {error}", json_path.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, report)
        .map_err(|error| format!("Failed to encode comparison report: {error}"))?;
    writer
        .write_all(b"\n")
        .map_err(|error| format!("Failed to finalize {}: {error}", json_path.display()))?;
    if let Some(path) = markdown_path {
        ensure_parent(path)?;
        fs::write(path, render_comparison_markdown(report))
            .map_err(|error| format!("Failed to write {}: {error}", path.display()))?;
    }
    Ok(())
}

pub fn render_comparison_markdown(report: &BenchmarkComparisonReport) -> String {
    let mut lines = vec![
        format!(
            "# {} → {}",
            report.baseline_benchmark_id, report.candidate_benchmark_id
        ),
        String::new(),
        format!("- Contract: `{}`", report.contract),
        format!(
            "- Budget: `{}` revision `{}`",
            report.budget_id, report.budget_revision
        ),
        format!("- Environment comparable: `{}`", report.environment_comparable),
        format!("- Matrix complete: `{}`", report.matrix_complete),
        format!("- Budget passed: `{}`", report.budget_passed),
        format!("- Promotion ready: `{}`", report.promotion_ready),
        String::new(),
        "## Before / After".to_string(),
        String::new(),
        "| Scale | Operation | Temperature | Baseline p50 ms | Candidate p50 ms | p50 reduction | p95 change | Candidate AIT/Git p50 | Candidate AIT/Git p95 |".to_string(),
        "| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |".to_string(),
    ];
    for cell in &report.cells {
        lines.push(format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            cell.fixture_scale,
            cell.operation,
            cell.temperature,
            format_ms(cell.baseline_ait_p50_ns),
            format_ms(cell.candidate_ait_p50_ns),
            format_percent(cell.p50_reduction_percent),
            format_percent(cell.p95_change_percent),
            format_ratio(cell.candidate_ait_git_p50_ratio),
            format_ratio(cell.candidate_ait_git_p95_ratio),
        ));
    }
    lines.extend([
        String::new(),
        "## Budget Results".to_string(),
        String::new(),
        "| Rule | Cell | Status | Reasons |".to_string(),
        "| --- | --- | --- | --- |".to_string(),
    ]);
    for result in &report.budget_results {
        lines.push(format!(
            "| {} | {} | {} | {} |",
            result.rule_id,
            result.cell_key.as_deref().unwrap_or("n/a"),
            result.status,
            if result.reasons.is_empty() {
                "—".to_string()
            } else {
                result.reasons.join("; ")
            }
        ));
    }
    if !report.blockers.is_empty() {
        lines.extend([
            String::new(),
            "## Promotion Blockers".to_string(),
            String::new(),
        ]);
        lines.extend(report.blockers.iter().map(|blocker| format!("- {blocker}")));
    }
    if !report.environment_differences.is_empty() {
        lines.extend([
            String::new(),
            "## Environment Differences".to_string(),
            String::new(),
        ]);
        lines.extend(
            report
                .environment_differences
                .iter()
                .map(|difference| format!("- {difference}")),
        );
    }
    if !report.limitations.is_empty() {
        lines.extend([String::new(), "## Limitations".to_string(), String::new()]);
        lines.extend(
            report
                .limitations
                .iter()
                .map(|limitation| format!("- {limitation}")),
        );
    }
    lines.push(String::new());
    lines.join("\n")
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path, label: &str) -> Result<T, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Failed to read {label} {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Failed to decode {label} {}: {error}", path.display()))
}

fn validate_budget(budget: &BenchmarkBudgetManifest) -> Result<(), String> {
    let mut errors = Vec::new();
    if budget.contract != BUDGET_CONTRACT {
        errors.push(format!("contract must be {BUDGET_CONTRACT}"));
    }
    if budget.budget_id.trim().is_empty() {
        errors.push("budget_id must not be empty".to_string());
    }
    if budget.revision.trim().is_empty() {
        errors.push("revision must not be empty".to_string());
    }
    if budget.rules.is_empty() {
        errors.push("rules must not be empty".to_string());
    }
    let mut ids = BTreeSet::new();
    for rule in &budget.rules {
        if rule.rule_id.trim().is_empty() || !ids.insert(rule.rule_id.as_str()) {
            errors.push(format!(
                "rule_id must be non-empty and unique: {}",
                rule.rule_id
            ));
        }
        if rule.operation.trim().is_empty() {
            errors.push(format!("rule {} operation must not be empty", rule.rule_id));
        }
        let thresholds = [
            rule.min_p50_reduction_percent,
            rule.max_p95_regression_percent,
            rule.max_candidate_ait_git_p50_ratio,
            rule.max_candidate_ait_git_p95_ratio,
        ];
        if thresholds.iter().all(Option::is_none) {
            errors.push(format!("rule {} has no threshold", rule.rule_id));
        }
        for value in thresholds.into_iter().flatten() {
            if !value.is_finite() || value < 0.0 {
                errors.push(format!(
                    "rule {} thresholds must be finite and non-negative",
                    rule.rule_id
                ));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("Invalid benchmark budget: {}", errors.join("; ")))
    }
}

fn index_cells<'a>(
    cells: &'a [CellReport],
    label: &str,
) -> Result<BTreeMap<String, &'a CellReport>, String> {
    let mut indexed = BTreeMap::new();
    for cell in cells {
        let key = cell_key(cell);
        if indexed.insert(key.clone(), cell).is_some() {
            return Err(format!(
                "{label} report contains duplicate comparison cell {key}"
            ));
        }
    }
    Ok(indexed)
}

fn cell_key(cell: &CellReport) -> String {
    format!(
        "{}/{}/{}/{}",
        cell.fixture_scale, cell.operation, cell.temperature, cell.sample_class
    )
}

fn compare_cell(
    key: &str,
    baseline: Option<&&CellReport>,
    candidate: Option<&&CellReport>,
) -> BeforeAfterCell {
    let dimensions = baseline.copied().or_else(|| candidate.copied()).unwrap();
    let baseline_ait = baseline.and_then(|cell| ait_times(cell));
    let candidate_ait = candidate.and_then(|cell| ait_times(cell));
    let baseline_ratios = baseline.and_then(|cell| cell.ait_vs_git.as_ref());
    let candidate_ratios = candidate.and_then(|cell| cell.ait_vs_git.as_ref());
    BeforeAfterCell {
        cell_key: key.to_string(),
        fixture_scale: dimensions.fixture_scale.clone(),
        operation: dimensions.operation.clone(),
        temperature: dimensions.temperature.clone(),
        sample_class: dimensions.sample_class.clone(),
        baseline_cell_id: baseline.map(|cell| cell.cell_id.clone()),
        candidate_cell_id: candidate.map(|cell| cell.cell_id.clone()),
        baseline_ait_p50_ns: baseline_ait.map(|times| times.0),
        baseline_ait_p95_ns: baseline_ait.map(|times| times.1),
        candidate_ait_p50_ns: candidate_ait.map(|times| times.0),
        candidate_ait_p95_ns: candidate_ait.map(|times| times.1),
        p50_reduction_percent: percent_reduction(
            baseline_ait.map(|times| times.0),
            candidate_ait.map(|times| times.0),
        ),
        p95_change_percent: percent_change(
            baseline_ait.map(|times| times.1),
            candidate_ait.map(|times| times.1),
        ),
        baseline_ait_git_p50_ratio: baseline_ratios
            .and_then(|comparison| comparison.p50_wall_time_ratio),
        baseline_ait_git_p95_ratio: baseline_ratios
            .and_then(|comparison| comparison.p95_wall_time_ratio),
        candidate_ait_git_p50_ratio: candidate_ratios
            .and_then(|comparison| comparison.p50_wall_time_ratio),
        candidate_ait_git_p95_ratio: candidate_ratios
            .and_then(|comparison| comparison.p95_wall_time_ratio),
    }
}

fn ait_times(cell: &CellReport) -> Option<(f64, f64)> {
    let summary = cell.subjects.iter().find(|subject| subject.role == "ait")?;
    let wall = summary.wall_time_ns.as_ref()?;
    Some((wall.p50, wall.p95))
}

fn rule_matches(rule: &BudgetRule, cell: &BeforeAfterCell) -> bool {
    rule.operation == cell.operation
        && rule
            .fixture_scale
            .as_ref()
            .is_none_or(|value| value == &cell.fixture_scale)
        && rule
            .temperature
            .as_ref()
            .is_none_or(|value| value == &cell.temperature)
        && rule
            .sample_class
            .as_ref()
            .is_none_or(|value| value == &cell.sample_class)
}

fn evaluate_rule(rule: &BudgetRule, cell: &BeforeAfterCell) -> BudgetResult {
    let mut missing = Vec::new();
    let mut failed = Vec::new();
    check_minimum(
        "p50 reduction percent",
        cell.p50_reduction_percent,
        rule.min_p50_reduction_percent,
        &mut missing,
        &mut failed,
    );
    check_maximum(
        "p95 regression percent",
        cell.p95_change_percent,
        rule.max_p95_regression_percent,
        &mut missing,
        &mut failed,
    );
    check_maximum(
        "candidate AIT/Git p50 ratio",
        cell.candidate_ait_git_p50_ratio,
        rule.max_candidate_ait_git_p50_ratio,
        &mut missing,
        &mut failed,
    );
    check_maximum(
        "candidate AIT/Git p95 ratio",
        cell.candidate_ait_git_p95_ratio,
        rule.max_candidate_ait_git_p95_ratio,
        &mut missing,
        &mut failed,
    );
    let (status, reasons) = if !missing.is_empty() {
        missing.extend(failed);
        ("missing", missing)
    } else if !failed.is_empty() {
        ("failed", failed)
    } else {
        ("passed", Vec::new())
    };
    BudgetResult {
        rule_id: rule.rule_id.clone(),
        cell_key: Some(cell.cell_key.clone()),
        status: status.to_string(),
        reasons,
    }
}

fn check_minimum(
    label: &str,
    actual: Option<f64>,
    threshold: Option<f64>,
    missing: &mut Vec<String>,
    failed: &mut Vec<String>,
) {
    if let Some(threshold) = threshold {
        match actual {
            Some(actual) if actual >= threshold => {}
            Some(actual) => failed.push(format!(
                "{label} {actual:.3} is below minimum {threshold:.3}"
            )),
            None => missing.push(format!("{label} is unavailable")),
        }
    }
}

fn check_maximum(
    label: &str,
    actual: Option<f64>,
    threshold: Option<f64>,
    missing: &mut Vec<String>,
    failed: &mut Vec<String>,
) {
    if let Some(threshold) = threshold {
        match actual {
            Some(actual) if actual <= threshold => {}
            Some(actual) => failed.push(format!(
                "{label} {actual:.3} exceeds maximum {threshold:.3}"
            )),
            None => missing.push(format!("{label} is unavailable")),
        }
    }
}

fn percent_reduction(baseline: Option<f64>, candidate: Option<f64>) -> Option<f64> {
    baseline.zip(candidate).and_then(|(baseline, candidate)| {
        (baseline > 0.0).then_some((baseline - candidate) / baseline * 100.0)
    })
}

fn percent_change(baseline: Option<f64>, candidate: Option<f64>) -> Option<f64> {
    baseline.zip(candidate).and_then(|(baseline, candidate)| {
        (baseline > 0.0).then_some((candidate - baseline) / baseline * 100.0)
    })
}

fn environment_differences(baseline: &EnvironmentPin, candidate: &EnvironmentPin) -> Vec<String> {
    let mut differences = Vec::new();
    compare_field("os", &baseline.os, &candidate.os, &mut differences);
    compare_field(
        "architecture",
        &baseline.architecture,
        &candidate.architecture,
        &mut differences,
    );
    compare_field(
        "filesystem",
        &baseline.filesystem,
        &candidate.filesystem,
        &mut differences,
    );
    compare_field(
        "storage_medium",
        &baseline.storage_medium,
        &candidate.storage_medium,
        &mut differences,
    );
    compare_field("cpu", &baseline.cpu, &candidate.cpu, &mut differences);
    if baseline.memory_bytes != candidate.memory_bytes {
        differences.push(format!(
            "memory_bytes differs: {} vs {}",
            baseline.memory_bytes, candidate.memory_bytes
        ));
    }
    compare_field(
        "rust_version",
        &baseline.rust_version,
        &candidate.rust_version,
        &mut differences,
    );
    compare_field(
        "git_version",
        &baseline.git_version,
        &candidate.git_version,
        &mut differences,
    );
    compare_field(
        "server_revision",
        &baseline.server_revision,
        &candidate.server_revision,
        &mut differences,
    );
    compare_field(
        "network_profile",
        &baseline.network_profile,
        &candidate.network_profile,
        &mut differences,
    );
    compare_field(
        "cache_drop_method",
        &baseline.cache_drop_method,
        &candidate.cache_drop_method,
        &mut differences,
    );
    if baseline.command_options != candidate.command_options {
        differences.push("command_options differ".to_string());
    }
    differences
}

fn compare_field(label: &str, baseline: &str, candidate: &str, output: &mut Vec<String>) {
    if baseline != candidate {
        output.push(format!("{label} differs: {baseline:?} vs {candidate:?}"));
    }
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    Ok(())
}

fn format_ms(value: Option<f64>) -> String {
    value
        .map(|value| format!("{:.3}", value / 1_000_000.0))
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_percent(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}%"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_ratio(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}x"))
        .unwrap_or_else(|| "n/a".to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::model::{CellComparison, SubjectReport};
    use crate::statistics::DistributionSummary;

    use super::*;

    #[test]
    fn focused_evidence_can_pass_budget_but_never_promotes() {
        let baseline = report("baseline", 100.0, 120.0, 4.0, false, "focused_slice");
        let candidate = report("candidate", 50.0, 100.0, 1.8, false, "focused_slice");
        let comparison = compare_reports(&baseline, &candidate, &budget()).unwrap();
        assert!(comparison.environment_comparable);
        assert!(comparison.matrix_complete);
        assert!(comparison.budget_passed);
        assert!(!comparison.promotion_ready);
        assert_eq!(comparison.cells[0].p50_reduction_percent, Some(50.0));
        assert!(comparison
            .blockers
            .iter()
            .any(|blocker| blocker.contains("not claim eligible")));
    }

    #[test]
    fn regression_and_environment_drift_are_explicit_blockers() {
        let baseline = report("baseline", 100.0, 100.0, 2.0, true, "full_matrix");
        let mut candidate = report("candidate", 80.0, 130.0, 2.5, true, "full_matrix");
        candidate.environment.cpu = "different CPU".to_string();
        let comparison = compare_reports(&baseline, &candidate, &budget()).unwrap();
        assert!(!comparison.environment_comparable);
        assert!(!comparison.budget_passed);
        assert!(!comparison.promotion_ready);
        assert_eq!(comparison.budget_results[0].status, "failed");
    }

    fn budget() -> BenchmarkBudgetManifest {
        BenchmarkBudgetManifest {
            contract: BUDGET_CONTRACT.to_string(),
            budget_id: "test-budget".to_string(),
            revision: "1".to_string(),
            rules: vec![BudgetRule {
                rule_id: "warm-status".to_string(),
                operation: "status_clean".to_string(),
                fixture_scale: Some("small".to_string()),
                temperature: Some("warm".to_string()),
                sample_class: Some("local".to_string()),
                min_p50_reduction_percent: Some(20.0),
                max_p95_regression_percent: Some(10.0),
                max_candidate_ait_git_p50_ratio: Some(2.0),
                max_candidate_ait_git_p95_ratio: None,
            }],
            limitations: vec![],
        }
    }

    fn report(
        benchmark_id: &str,
        p50: f64,
        p95: f64,
        ratio: f64,
        claim_eligible: bool,
        campaign_scope: &str,
    ) -> BenchmarkReport {
        let environment = EnvironmentPin {
            captured_at: "2026-07-19T00:00:00Z".to_string(),
            os: "macOS".to_string(),
            architecture: "arm64".to_string(),
            filesystem: "APFS".to_string(),
            storage_medium: "NVMe".to_string(),
            cpu: "test CPU".to_string(),
            memory_bytes: 16,
            rust_version: "rustc test".to_string(),
            git_version: "git test".to_string(),
            ait_version: benchmark_id.to_string(),
            repository_snapshot: benchmark_id.to_string(),
            server_revision: "none".to_string(),
            network_profile: "none".to_string(),
            cache_drop_method: "none".to_string(),
            command_options: BTreeMap::new(),
        };
        let wall = DistributionSummary {
            sample_count: 50,
            min: p50,
            p50,
            p95,
            max: p95,
            median_absolute_deviation: 0.0,
            p50_bootstrap_ci95: [p50, p50],
            p95_bootstrap_ci95: [p95, p95],
            quantile_method: "R-7".to_string(),
            bootstrap_resamples: 1_000,
        };
        let subject = SubjectReport {
            subject_id: format!("ait-{benchmark_id}"),
            role: "ait".to_string(),
            measured_sample_count: 50,
            failure_count: 0,
            wall_time_ns: Some(wall),
            cpu_time_ns: None,
            peak_rss_bytes: None,
            io_read_bytes: None,
            io_write_bytes: None,
            transferred_bytes: None,
            server_latency_ns: None,
            server_health_failure_count: 0,
        };
        BenchmarkReport {
            contract: REPORT_CONTRACT.to_string(),
            benchmark_id: benchmark_id.to_string(),
            protocol_revision: "v1".to_string(),
            generated_at: "2026-07-19T00:00:00Z".to_string(),
            manifest_digest: "sha256:test".to_string(),
            raw_jsonl_path: "raw.jsonl".to_string(),
            environment,
            evidence_class: "measured".to_string(),
            protocol_conformant: claim_eligible,
            campaign_scope: campaign_scope.to_string(),
            claim_eligible,
            total_failure_count: 0,
            cells: vec![CellReport {
                cell_id: format!("{benchmark_id}-status"),
                fixture_id: "small".to_string(),
                fixture_scale: "small".to_string(),
                operation: "status_clean".to_string(),
                temperature: "warm".to_string(),
                sample_class: "local".to_string(),
                subjects: vec![subject],
                ait_vs_git: Some(CellComparison {
                    candidate_subject_id: format!("ait-{benchmark_id}"),
                    baseline_subject_id: "git".to_string(),
                    p50_wall_time_ratio: Some(ratio),
                    p95_wall_time_ratio: Some(ratio),
                }),
            }],
            limitations: vec![],
        }
    }
}
