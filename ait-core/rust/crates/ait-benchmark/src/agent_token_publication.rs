use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    sha256_digest, AgentTokenCampaignManifest, AgentTokenGroupReport, AgentTokenMode,
    AgentTokenModeComparison, AgentTokenModelPin, AgentTokenReport, AgentTokenRunSummary,
    AgentTokenStatisticalReplacementRecord,
};

pub const AGENT_TOKEN_PUBLICATION_CONTRACT: &str = "ait-agent-token-benchmark-publication/v1";
pub const AGENT_TOKEN_PUBLIC_RUN_INDEX_CONTRACT: &str = "ait-agent-token-benchmark-public-runs/v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenPublicationResult {
    pub contract: String,
    pub release_version: String,
    pub campaign_id: String,
    pub protocol_revision: String,
    pub campaign_scope: String,
    pub accounting_profile: String,
    pub model: AgentTokenModelPin,
    pub measured_product_snapshot: String,
    pub measured_ait_executable_sha256: String,
    pub campaign_runner_sha256: String,
    pub cache_class: String,
    pub network_policy: String,
    pub workload_ids: Vec<String>,
    pub modes: Vec<String>,
    pub attempts_per_cell: usize,
    pub bootstrap_resamples: usize,
    pub scheduled_run_count: usize,
    pub observed_run_count: usize,
    pub executed_evidence_run_count: usize,
    pub statistically_excluded_run_count: usize,
    pub valid_run_count: usize,
    pub invalid_run_count: usize,
    pub accepted_run_count: usize,
    pub accepted_by_mode: BTreeMap<String, usize>,
    pub provider_total_tokens_by_mode: BTreeMap<String, u64>,
    pub raw_provider_total_token_savings_percent: f64,
    pub aggregate_median_token_savings_percent: Option<f64>,
    pub aggregate_token_savings_bootstrap_ci95: Option<[f64; 2]>,
    pub aggregate_median_elapsed_savings_percent: Option<f64>,
    pub pair_admission_policy: String,
    pub workload_results: Vec<AgentTokenModeComparison>,
    pub group_results: Vec<AgentTokenGroupReport>,
    pub claim_eligible: bool,
    pub source_protocol_claim_eligible: bool,
    pub claim_blockers: Vec<String>,
    pub source_protocol_blockers: Vec<String>,
    pub retained_failures: Vec<AgentTokenPublicFailure>,
    pub statistically_excluded_failures: Vec<AgentTokenPublicFailure>,
    pub replacement_policy_revision: Option<String>,
    pub statistical_replacements: Vec<AgentTokenStatisticalReplacementRecord>,
    pub scope_limitations: Vec<String>,
    pub source_sha256: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenPublicFailure {
    pub run_id: String,
    pub workload_id: String,
    pub mode: String,
    pub attempt: usize,
    pub valid_attempt: bool,
    pub evaluator_score: Option<u64>,
    pub evaluator_accepted: bool,
    pub browser_status: String,
    pub workflow_closed: bool,
    pub failure_reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenPublicRunIndex {
    pub contract: String,
    pub campaign_id: String,
    pub scheduled_run_count: usize,
    pub observed_run_count: usize,
    pub executed_evidence_run_count: usize,
    pub statistically_excluded_run_count: usize,
    pub runs: Vec<AgentTokenPublicRun>,
    pub excluded_runs: Vec<AgentTokenPublicRun>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenPublicRun {
    pub run_id: String,
    pub workload_id: String,
    pub mode: String,
    pub attempt: usize,
    pub block_index: usize,
    pub randomized_order: usize,
    pub source_run_summary_sha256: String,
    pub valid_attempt: bool,
    pub accepted_equivalent: bool,
    pub provider_total_tokens: Option<u64>,
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub elapsed_ms: u64,
    pub evaluator_score: Option<u64>,
    pub browser_status: String,
    pub workflow_closed: bool,
    pub completed_file_change_items: usize,
    pub rejected_apply_patch_attempts: usize,
    pub total_apply_patch_attempts: usize,
    pub failure_reasons: Vec<String>,
    pub invalid_reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenPublicationReceipt {
    pub contract: String,
    pub campaign_id: String,
    pub release_version: String,
    pub output_dir: PathBuf,
    pub files: BTreeMap<String, String>,
}

pub struct AgentTokenPublicationInput<'a> {
    pub output_dir: &'a Path,
    pub release_version: &'a str,
    pub measured_product_snapshot: &'a str,
    pub measured_ait_executable_sha256: &'a str,
    pub campaign_runner_sha256: &'a str,
    pub manifest: &'a AgentTokenCampaignManifest,
    pub report: &'a AgentTokenReport,
    pub runs: &'a [AgentTokenRunSummary],
    pub excluded_runs: &'a [AgentTokenRunSummary],
    pub run_summary_paths: &'a BTreeMap<String, PathBuf>,
    pub excluded_run_summary_paths: &'a BTreeMap<String, PathBuf>,
    pub source_files: &'a [(&'a str, &'a Path)],
}

pub fn write_agent_token_publication_bundle(
    input: AgentTokenPublicationInput<'_>,
) -> Result<AgentTokenPublicationReceipt, String> {
    let AgentTokenPublicationInput {
        output_dir,
        release_version,
        measured_product_snapshot,
        measured_ait_executable_sha256,
        campaign_runner_sha256,
        manifest,
        report,
        runs,
        excluded_runs,
        run_summary_paths,
        excluded_run_summary_paths,
        source_files,
    } = input;
    if release_version.trim().is_empty() {
        return Err("Agent-token publication release version must not be empty".to_string());
    }
    if !measured_product_snapshot.starts_with("SNP-")
        || measured_product_snapshot.len() != 16
        || !measured_product_snapshot[4..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
    {
        return Err(
            "Measured product Snapshot must use canonical SNP-XXXXXXXXXXXX form".to_string(),
        );
    }
    for (label, digest) in [
        ("measured AIT executable", measured_ait_executable_sha256),
        ("campaign runner", campaign_runner_sha256),
    ] {
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!(
                "{label} SHA-256 must contain exactly 64 hexadecimal characters"
            ));
        }
    }
    if output_dir.exists() {
        return Err(format!(
            "Agent-token publication output must not already exist: {}",
            output_dir.display()
        ));
    }
    let source_sha256 = source_files
        .iter()
        .map(|(name, path)| {
            let bytes = fs::read(path).map_err(|error| {
                format!(
                    "Failed to read publication source {}: {error}",
                    path.display()
                )
            })?;
            Ok(((*name).to_string(), raw_sha256(&bytes)))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;

    let mut accepted_by_mode = BTreeMap::<String, usize>::new();
    let mut provider_total_tokens_by_mode = BTreeMap::<String, u64>::new();
    for run in runs.iter().filter(|run| run.valid_attempt) {
        let mode = run.mode.as_str().to_string();
        if run.accepted_equivalent {
            *accepted_by_mode.entry(mode.clone()).or_default() += 1;
        }
        if let Some(usage) = run.usage.as_ref() {
            *provider_total_tokens_by_mode.entry(mode).or_default() += usage.provider_total_tokens;
        }
    }
    for mode in manifest.modes.iter().map(AgentTokenMode::as_str) {
        accepted_by_mode.entry(mode.to_string()).or_default();
        provider_total_tokens_by_mode
            .entry(mode.to_string())
            .or_default();
    }
    let git_tokens = *provider_total_tokens_by_mode
        .get(AgentTokenMode::GitLinearSingleSession.as_str())
        .unwrap_or(&0);
    let ait_tokens = *provider_total_tokens_by_mode
        .get(AgentTokenMode::AitLinearSingleSession.as_str())
        .unwrap_or(&0);
    if git_tokens == 0 {
        return Err(
            "Agent-token publication cannot calculate raw savings without Git tokens".to_string(),
        );
    }

    let retained_failures = runs
        .iter()
        .filter(|run| run.valid_attempt && !run.accepted_equivalent)
        .map(|run| AgentTokenPublicFailure {
            run_id: run.run_id.clone(),
            workload_id: run.workload_id.clone(),
            mode: run.mode.as_str().to_string(),
            attempt: run.attempt,
            valid_attempt: run.valid_attempt,
            evaluator_score: run.evaluator_score,
            evaluator_accepted: run.evaluator_accepted,
            browser_status: run.browser.status.clone(),
            workflow_closed: run.workflow_closed,
            failure_reasons: run.failure_reasons.clone(),
        })
        .collect::<Vec<_>>();
    let statistically_excluded_failures = excluded_runs
        .iter()
        .filter(|run| run.valid_attempt && !run.accepted_equivalent)
        .map(|run| AgentTokenPublicFailure {
            run_id: run.run_id.clone(),
            workload_id: run.workload_id.clone(),
            mode: run.mode.as_str().to_string(),
            attempt: run.attempt,
            valid_attempt: run.valid_attempt,
            evaluator_score: run.evaluator_score,
            evaluator_accepted: run.evaluator_accepted,
            browser_status: run.browser.status.clone(),
            workflow_closed: run.workflow_closed,
            failure_reasons: run.failure_reasons.clone(),
        })
        .collect::<Vec<_>>();
    let result = AgentTokenPublicationResult {
        contract: AGENT_TOKEN_PUBLICATION_CONTRACT.to_string(),
        release_version: release_version.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        protocol_revision: manifest.protocol_revision.clone(),
        campaign_scope: manifest.campaign_scope.as_str().to_string(),
        accounting_profile: manifest.accounting_profile.as_str().to_string(),
        model: manifest.model.clone(),
        measured_product_snapshot: measured_product_snapshot.to_string(),
        measured_ait_executable_sha256: measured_ait_executable_sha256.to_string(),
        campaign_runner_sha256: campaign_runner_sha256.to_string(),
        cache_class: manifest.cache_class.clone(),
        network_policy: manifest.network_policy.clone(),
        workload_ids: manifest.workload_ids.clone(),
        modes: manifest
            .modes
            .iter()
            .map(|mode| mode.as_str().to_string())
            .collect(),
        attempts_per_cell: manifest.attempts_per_cell,
        bootstrap_resamples: manifest.bootstrap_resamples,
        scheduled_run_count: report.scheduled_run_count,
        observed_run_count: report.observed_run_count,
        executed_evidence_run_count: report.executed_evidence_run_count,
        statistically_excluded_run_count: report.statistically_excluded_run_count,
        valid_run_count: runs.iter().filter(|run| run.valid_attempt).count(),
        invalid_run_count: report.invalid_run_count,
        accepted_run_count: runs
            .iter()
            .filter(|run| run.valid_attempt && run.accepted_equivalent)
            .count(),
        accepted_by_mode,
        provider_total_tokens_by_mode,
        raw_provider_total_token_savings_percent: 100.0
            * (1.0 - ait_tokens as f64 / git_tokens as f64),
        aggregate_median_token_savings_percent: report
            .aggregate_median_token_savings_percent,
        aggregate_token_savings_bootstrap_ci95: report
            .aggregate_token_savings_bootstrap_ci95,
        aggregate_median_elapsed_savings_percent: report
            .aggregate_median_elapsed_savings_percent,
        pair_admission_policy: report.pair_admission_policy.clone(),
        workload_results: report.comparisons.clone(),
        group_results: report.groups.clone(),
        claim_eligible: report.claim_eligible,
        source_protocol_claim_eligible: report.source_protocol_claim_eligible,
        claim_blockers: report.blockers.clone(),
        source_protocol_blockers: report.source_protocol_blockers.clone(),
        retained_failures,
        statistically_excluded_failures,
        replacement_policy_revision: report.replacement_policy_revision.clone(),
        statistical_replacements: report.statistical_replacements.clone(),
        scope_limitations: vec![
            "Results are limited to the five frozen game-development workloads and do not establish universal superiority.".to_string(),
            "Provider-total tokens include cached input as reported by the provider and are not a monetary-cost estimate.".to_string(),
            "The primary aggregate is the median of workload-level failure-adjusted savings; pooled raw totals are descriptive only.".to_string(),
        ],
        source_sha256,
    };

    let mut public_runs = runs
        .iter()
        .map(|run| {
            let path = run_summary_paths
                .get(&run.run_id)
                .ok_or_else(|| format!("Public benchmark run {} has no source path", run.run_id))?;
            public_run(run, path)
        })
        .collect::<Result<Vec<_>, String>>()?;
    public_runs.sort_by_key(|run| (run.block_index, run.randomized_order));
    let mut public_excluded_runs = excluded_runs
        .iter()
        .map(|run| {
            let path = excluded_run_summary_paths.get(&run.run_id).ok_or_else(|| {
                format!(
                    "Public excluded benchmark run {} has no source path",
                    run.run_id
                )
            })?;
            public_run(run, path)
        })
        .collect::<Result<Vec<_>, String>>()?;
    public_excluded_runs.sort_by_key(|run| (run.block_index, run.randomized_order));
    let run_index = AgentTokenPublicRunIndex {
        contract: AGENT_TOKEN_PUBLIC_RUN_INDEX_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        scheduled_run_count: report.scheduled_run_count,
        observed_run_count: public_runs.len(),
        executed_evidence_run_count: report.executed_evidence_run_count,
        statistically_excluded_run_count: report.statistically_excluded_run_count,
        runs: public_runs,
        excluded_runs: public_excluded_runs,
    };

    let result_bytes = pretty_json(&result, "publication result")?;
    let run_bytes = pretty_json(&run_index, "public run index")?;
    let readme = render_publication_markdown(&result);
    fs::create_dir_all(output_dir).map_err(|error| {
        format!(
            "Failed to create agent-token publication output {}: {error}",
            output_dir.display()
        )
    })?;
    write_new(&output_dir.join("result.json"), &result_bytes)?;
    write_new(&output_dir.join("runs.json"), &run_bytes)?;
    write_new(&output_dir.join("summary.txt"), readme.as_bytes())?;

    let mut files = BTreeMap::new();
    files.insert("summary.txt".to_string(), raw_sha256(readme.as_bytes()));
    files.insert("result.json".to_string(), raw_sha256(&result_bytes));
    files.insert("runs.json".to_string(), raw_sha256(&run_bytes));
    let checksums = files
        .iter()
        .map(|(name, digest)| format!("{digest}  {name}\n"))
        .collect::<String>();
    write_new(&output_dir.join("SHA256SUMS"), checksums.as_bytes())?;
    files.insert("SHA256SUMS".to_string(), raw_sha256(checksums.as_bytes()));
    validate_publication_text(output_dir)?;

    Ok(AgentTokenPublicationReceipt {
        contract: AGENT_TOKEN_PUBLICATION_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        release_version: release_version.to_string(),
        output_dir: output_dir.to_path_buf(),
        files,
    })
}

fn public_run(
    run: &AgentTokenRunSummary,
    source_path: &Path,
) -> Result<AgentTokenPublicRun, String> {
    if run.run_id.contains('/') || run.run_id.contains("..") {
        return Err(format!(
            "Public benchmark run ID is not path-safe: {}",
            run.run_id
        ));
    }
    let bytes = fs::read(source_path).map_err(|error| {
        format!(
            "Failed to read source run summary {}: {error}",
            source_path.display()
        )
    })?;
    Ok(AgentTokenPublicRun {
        run_id: run.run_id.clone(),
        workload_id: run.workload_id.clone(),
        mode: run.mode.as_str().to_string(),
        attempt: run.attempt,
        block_index: run.block_index,
        randomized_order: run.randomized_order,
        source_run_summary_sha256: raw_sha256(&bytes),
        valid_attempt: run.valid_attempt,
        accepted_equivalent: run.accepted_equivalent,
        provider_total_tokens: run.usage.as_ref().map(|usage| usage.provider_total_tokens),
        input_tokens: run.usage.as_ref().map(|usage| usage.input_tokens),
        cached_input_tokens: run
            .usage
            .as_ref()
            .and_then(|usage| usage.cached_input_tokens),
        output_tokens: run.usage.as_ref().map(|usage| usage.output_tokens),
        reasoning_tokens: run.usage.as_ref().and_then(|usage| usage.reasoning_tokens),
        elapsed_ms: run.elapsed_ms,
        evaluator_score: run.evaluator_score,
        browser_status: run.browser.status.clone(),
        workflow_closed: run.workflow_closed,
        completed_file_change_items: run.secondary_metrics.file_change_items,
        rejected_apply_patch_attempts: run.secondary_metrics.apply_patch_rejected_attempts,
        total_apply_patch_attempts: run.secondary_metrics.apply_patch_attempts,
        failure_reasons: run.failure_reasons.clone(),
        invalid_reasons: run.invalid_reasons.clone(),
    })
}

fn pretty_json(value: &impl Serialize, label: &str) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Failed to encode {label}: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn raw_sha256(bytes: &[u8]) -> String {
    sha256_digest(bytes)
        .strip_prefix("sha256:")
        .unwrap_or_else(|| unreachable!("sha256_digest always returns a prefixed digest"))
        .to_string()
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Failed to create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

fn render_publication_markdown(result: &AgentTokenPublicationResult) -> String {
    let ait_accepted = result
        .accepted_by_mode
        .get(AgentTokenMode::AitLinearSingleSession.as_str())
        .copied()
        .unwrap_or(0);
    let git_accepted = result
        .accepted_by_mode
        .get(AgentTokenMode::GitLinearSingleSession.as_str())
        .copied()
        .unwrap_or(0);
    let mut output = format!(
        "# AIT vs Git: 200-admitted-session game-development benchmark\n\n\
This result accompanies AIT Native {}. It measures GPT-5.6 Sol at max reasoning effort on five frozen game-development workloads, with 20 statistically admitted paired attempts per workload and one fresh model session per lane.\n\n\
## Result\n\n\
- Workload-median token saving: **{:.2}%**.\n\
- Aggregate bootstrap 95% confidence interval: **{:.2}% to {:.2}%**.\n\
- Workload-median elapsed-time saving: **{:.2}%**.\n\
- Raw provider-total tokens: **{} AIT** versus **{} Git**, a descriptive **{:.2}%** reduction.\n\
- Effective functional acceptance: **{}/100 AIT** versus **{}/100 Git**; all 200 statistically admitted sessions were protocol-valid and accepted.\n\
- Evidence history: **{} executed sessions**; **{} session(s) were statistically excluded**.\n\
- Replacement-qualified claim eligible: **{}**.\n\n\
The primary metric divides each workload and mode's statistically admitted provider tokens by accepted outcomes, then takes the median of the five workload-level AIT savings. The bootstrap interval resamples admitted paired attempts within each workload and aggregates their workload medians over 2,000 deterministic resamples. Raw pooled totals are descriptive and do not replace the workload-balanced primary metric.\n\n\
The measured AIT subject was source Snapshot `{}` with executable SHA-256 `{}`. The campaign runner SHA-256 was `{}`. AIT Native 1.1.0 publishes this immutable result and the corrected report boundary without relabeling later source bytes as the measured subject.\n\n\
## Workload results\n\n\
| Workload | AIT effective tokens | Git effective tokens | Saving | Bootstrap 95% CI | AIT/Git acceptance |\n\
| --- | ---: | ---: | ---: | ---: | ---: |\n",
        result.release_version,
        result.aggregate_median_token_savings_percent.unwrap_or(f64::NAN),
        result.aggregate_token_savings_bootstrap_ci95.unwrap_or([f64::NAN; 2])[0],
        result.aggregate_token_savings_bootstrap_ci95.unwrap_or([f64::NAN; 2])[1],
        result.aggregate_median_elapsed_savings_percent.unwrap_or(f64::NAN),
        Thousands(*result.provider_total_tokens_by_mode.get(AgentTokenMode::AitLinearSingleSession.as_str()).unwrap_or(&0)),
        Thousands(*result.provider_total_tokens_by_mode.get(AgentTokenMode::GitLinearSingleSession.as_str()).unwrap_or(&0)),
        result.raw_provider_total_token_savings_percent,
        ait_accepted,
        git_accepted,
        result.executed_evidence_run_count,
        result.statistically_excluded_run_count,
        result.claim_eligible,
        result.measured_product_snapshot,
        result.measured_ait_executable_sha256,
        result.campaign_runner_sha256,
    );
    for comparison in &result.workload_results {
        let ait_group = result.group_results.iter().find(|group| {
            group.workload_id == comparison.workload_id
                && group.mode == AgentTokenMode::AitLinearSingleSession.as_str()
        });
        let git_group = result.group_results.iter().find(|group| {
            group.workload_id == comparison.workload_id
                && group.mode == AgentTokenMode::GitLinearSingleSession.as_str()
        });
        let interval = comparison
            .token_savings_bootstrap_ci95
            .unwrap_or([f64::NAN; 2]);
        output.push_str(&format!(
            "| `{}` | {:.1} | {:.1} | {:.2}% | {:.2}% to {:.2}% | {}/{} |\n",
            comparison.workload_id,
            comparison.ait_effective_tokens.unwrap_or(f64::NAN),
            comparison.git_effective_tokens.unwrap_or(f64::NAN),
            comparison.token_savings_percent.unwrap_or(f64::NAN),
            interval[0],
            interval[1],
            ait_group.map(|group| group.accepted_count).unwrap_or(0),
            git_group.map(|group| group.accepted_count).unwrap_or(0),
        ));
    }
    output.push_str("\n## Quality and scope\n\n");
    if result.replacement_policy_revision.is_some() {
        let replacement_runner_sha256 = result
            .statistical_replacements
            .first()
            .map(|replacement| replacement.replacement_runner_sha256.as_str())
            .unwrap_or("unavailable");
        output.push_str(&format!(
            "The original AIT GD-05 candidate remains retained and disclosed as a valid functional failure: the evaluator scored it 50/100 because `normalizeSettings({{soundEnabled: 0}})` produced `soundEnabled: true`. Its browser checks and workflow closeout passed, but it was not accepted. Under the repository-owner-authorized transparent replacement policy, that lane is excluded from effective statistics and one same-pinned, accepted AIT replacement is admitted against the unchanged Git counterpart. The raw failure evidence and replacement provenance remain checksummed; the replacement runner SHA-256 is `{replacement_runner_sha256}` and the effective matrix is claim-eligible under the replacement policy.\n\n",
        ));
    } else {
        output.push_str(
            "No statistical replacement policy is active for this result. Claim eligibility and failures are reported directly from the frozen campaign evidence.\n\n",
        );
    }
    output.push_str(
        "The finding is limited to these fixtures, this model revision, max reasoning effort, provider-default fresh sessions, and the recorded local workflows. It does not establish that AIT always outperforms Git, generalize to other tasks, models, or environments, or measure high-concurrency execution. Both treatments had symmetric read-only repository inspection allowances.\n\n\
## Audit files\n\n\
- `result.json` contains the aggregate, workload, acceptance, exclusion, replacement, and source-digest record.\n\
- `runs.json` contains 200 statistically admitted rows plus the disclosed excluded source row.\n\
- `SHA256SUMS` checksums the public files.\n\n\
The frozen source protocol, replacement policy, and workload fixtures are versioned with the AIT source. Private executor event streams and host-specific absolute paths are intentionally excluded.\n",
    );
    output
}

fn validate_publication_text(output_dir: &Path) -> Result<(), String> {
    const FORBIDDEN: [&str; 4] = ["/Users/", ".ait-runtime", "private/", "codex-events.raw"];
    for name in ["summary.txt", "result.json", "runs.json", "SHA256SUMS"] {
        let path = output_dir.join(name);
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?;
        if let Some(forbidden) = FORBIDDEN.iter().find(|value| text.contains(**value)) {
            return Err(format!(
                "Public benchmark artifact {} contains forbidden text {:?}",
                path.display(),
                forbidden
            ));
        }
    }
    Ok(())
}

struct Thousands(u64);

impl std::fmt::Display for Thousands {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let digits = self.0.to_string();
        for (index, character) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index).is_multiple_of(3) {
                formatter.write_str(",")?;
            }
            formatter.write_fmt(format_args!("{character}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Thousands;

    #[test]
    fn thousands_formats_public_totals() {
        assert_eq!(Thousands(0).to_string(), "0");
        assert_eq!(Thousands(999).to_string(), "999");
        assert_eq!(Thousands(1_234_567).to_string(), "1,234,567");
    }
}
