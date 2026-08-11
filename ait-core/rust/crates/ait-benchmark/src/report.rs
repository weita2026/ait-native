use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use chrono::Utc;

use crate::model::{
    BenchmarkManifest, BenchmarkReport, CellComparison, CellReport, RawFooter, RawHeader,
    RawRecord, SampleClass, SampleRecord, SubjectReport,
};
use crate::statistics::{summarize_samples, DistributionSummary};
use crate::REPORT_CONTRACT;

pub fn build_report(
    manifest: &BenchmarkManifest,
    manifest_digest: &str,
    raw_jsonl_path: &Path,
) -> Result<BenchmarkReport, String> {
    let (header, samples, footer) = read_raw_records(raw_jsonl_path)?;
    if header.contract != crate::RAW_CONTRACT {
        return Err(format!(
            "Raw JSONL header contract must be {}, got {}",
            crate::RAW_CONTRACT,
            header.contract
        ));
    }
    if header.benchmark_id != manifest.benchmark_id
        || header.protocol_revision != manifest.protocol_revision
        || header.manifest_digest != manifest_digest
        || header.campaign_scope != manifest.campaign_scope.as_str()
    {
        return Err(
            "Raw JSONL header does not match benchmark id, protocol revision, campaign scope, and manifest digest".to_string(),
        );
    }
    if footer.contract != crate::RAW_CONTRACT || footer.benchmark_id != manifest.benchmark_id {
        return Err(
            "Raw JSONL footer contract or benchmark_id does not match manifest".to_string(),
        );
    }
    if footer.sample_count != samples.len() {
        return Err(format!(
            "Raw footer sample_count {} does not match {} sample records",
            footer.sample_count,
            samples.len()
        ));
    }

    let total_failure_count = samples.iter().filter(|sample| !sample.success).count();
    let mut groups = BTreeMap::<(String, String), Vec<&SampleRecord>>::new();
    for sample in samples.iter().filter(|sample| !sample.warmup) {
        if sample.benchmark_id != manifest.benchmark_id
            || sample.protocol_revision != manifest.protocol_revision
            || sample.contract != crate::RAW_CONTRACT
        {
            return Err("Raw JSONL contains a sample from another benchmark contract".to_string());
        }
        groups
            .entry((sample.cell_id.clone(), sample.subject_id.clone()))
            .or_default()
            .push(sample);
    }

    let mut cells = Vec::with_capacity(manifest.cells.len());
    let mut complete_sample_counts = true;
    for (cell_index, cell) in manifest.cells.iter().enumerate() {
        let fixture = manifest
            .fixtures
            .iter()
            .find(|fixture| fixture.fixture_id == cell.fixture_id)
            .ok_or_else(|| format!("Unknown fixture {}", cell.fixture_id))?;
        let expected_count = match cell.sample_class {
            SampleClass::Local => manifest.sampling.measured_local_iterations,
            SampleClass::ProcessNetwork => manifest.sampling.measured_cold_iterations,
        };
        let mut subjects = Vec::with_capacity(cell.subjects.len());
        for (subject_index, subject) in cell.subjects.iter().enumerate() {
            let group = groups
                .get(&(cell.cell_id.clone(), subject.subject_id.clone()))
                .cloned()
                .unwrap_or_default();
            if header.protocol_conformant && group.len() != expected_count {
                complete_sample_counts = false;
            }
            subjects.push(summarize_subject(
                subject.subject_id.as_str(),
                subject.role.as_str(),
                &group,
                manifest.bootstrap_resamples,
                manifest.seed ^ stable_seed(cell_index, subject_index, &cell.cell_id),
            )?);
        }
        let comparison = comparison_for_subjects(&subjects);
        cells.push(CellReport {
            cell_id: cell.cell_id.clone(),
            fixture_id: fixture.fixture_id.clone(),
            fixture_scale: fixture.scale.as_str().to_string(),
            operation: cell.operation.clone(),
            temperature: cell.temperature.as_str().to_string(),
            sample_class: cell.sample_class.as_str().to_string(),
            subjects,
            ait_vs_git: comparison,
        });
    }

    Ok(BenchmarkReport {
        contract: REPORT_CONTRACT.to_string(),
        benchmark_id: manifest.benchmark_id.clone(),
        protocol_revision: manifest.protocol_revision.clone(),
        generated_at: Utc::now().to_rfc3339(),
        manifest_digest: manifest_digest.to_string(),
        raw_jsonl_path: raw_jsonl_path.display().to_string(),
        environment: header.environment,
        evidence_class: header.evidence_class,
        protocol_conformant: header.protocol_conformant,
        campaign_scope: header.campaign_scope,
        claim_eligible: header.protocol_conformant
            && manifest.campaign_scope.claim_eligible()
            && complete_sample_counts
            && total_failure_count == 0,
        total_failure_count,
        cells,
        limitations: manifest.limitations.clone(),
    })
}

pub fn write_report(
    report: &BenchmarkReport,
    json_path: &Path,
    markdown_path: Option<&Path>,
) -> Result<(), String> {
    ensure_parent(json_path)?;
    let file = File::create(json_path)
        .map_err(|error| format!("Failed to create report {}: {error}", json_path.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, report)
        .map_err(|error| format!("Failed to encode benchmark report: {error}"))?;
    writer
        .write_all(b"\n")
        .map_err(|error| format!("Failed to finalize benchmark report: {error}"))?;
    if let Some(path) = markdown_path {
        ensure_parent(path)?;
        fs::write(path, render_markdown(report)).map_err(|error| {
            format!(
                "Failed to write Markdown report {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

pub fn render_markdown(report: &BenchmarkReport) -> String {
    let mut lines = vec![
        format!("# {}", report.benchmark_id),
        String::new(),
        "Generated from authoritative raw JSONL by `ait-benchmark`.".to_string(),
        String::new(),
        format!("- Contract: `{}`", report.contract),
        format!("- Protocol revision: `{}`", report.protocol_revision),
        format!("- Evidence class: `{}`", report.evidence_class),
        format!("- Campaign scope: `{}`", report.campaign_scope),
        format!("- Protocol conformant: `{}`", report.protocol_conformant),
        format!("- Claim eligible: `{}`", report.claim_eligible),
        format!("- Measured failures: `{}`", report.total_failure_count),
        format!("- Raw JSONL: `{}`", report.raw_jsonl_path),
        String::new(),
        "## Environment".to_string(),
        String::new(),
        "| OS/arch | Filesystem/storage | CPU | Memory | AIT | Git | Rust | Repository Snapshot | Server | Network | Cache drop |".to_string(),
        "| --- | --- | --- | ---: | --- | --- | --- | --- | --- | --- | --- |".to_string(),
        format!(
            "| {}/{} | {}/{} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            report.environment.os,
            report.environment.architecture,
            report.environment.filesystem,
            report.environment.storage_medium,
            report.environment.cpu,
            report.environment.memory_bytes,
            report.environment.ait_version,
            report.environment.git_version,
            report.environment.rust_version,
            report.environment.repository_snapshot,
            report.environment.server_revision,
            report.environment.network_profile,
            report.environment.cache_drop_method,
        ),
        String::new(),
        "## p50 / p95 Results".to_string(),
        String::new(),
        "| Scale | Operation | Temperature | Subject | n | Failures | p50 ms | p95 ms | MAD ms | p50 CI95 ms | p95 CI95 ms | Peak RSS p95 MiB |".to_string(),
        "| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- | --- | ---: |".to_string(),
    ];
    for cell in &report.cells {
        for subject in &cell.subjects {
            let wall = subject.wall_time_ns.as_ref();
            let rss = subject.peak_rss_bytes.as_ref();
            lines.push(format!(
                "| {} | {} | {} | {} ({}) | {} | {} | {} | {} | {} | {} | {} | {} |",
                cell.fixture_scale,
                cell.operation,
                cell.temperature,
                subject.subject_id,
                subject.role,
                subject.measured_sample_count,
                subject.failure_count,
                format_ms(wall.map(|value| value.p50)),
                format_ms(wall.map(|value| value.p95)),
                format_ms(wall.map(|value| value.median_absolute_deviation)),
                format_ci_ms(wall.map(|value| value.p50_bootstrap_ci95)),
                format_ci_ms(wall.map(|value| value.p95_bootstrap_ci95)),
                rss.map(|value| format!("{:.2}", value.p95 / 1024.0 / 1024.0))
                    .unwrap_or_else(|| "n/a".to_string()),
            ));
        }
    }
    lines.extend([
        String::new(),
        "## AIT / Git Ratios".to_string(),
        String::new(),
        "| Scale | Operation | Temperature | AIT | Git | p50 ratio | p95 ratio |".to_string(),
        "| --- | --- | --- | --- | --- | ---: | ---: |".to_string(),
    ]);
    for cell in &report.cells {
        if let Some(comparison) = &cell.ait_vs_git {
            lines.push(format!(
                "| {} | {} | {} | {} | {} | {} | {} |",
                cell.fixture_scale,
                cell.operation,
                cell.temperature,
                comparison.candidate_subject_id,
                comparison.baseline_subject_id,
                format_ratio(comparison.p50_wall_time_ratio),
                format_ratio(comparison.p95_wall_time_ratio),
            ));
        }
    }
    if !report.limitations.is_empty() {
        lines.extend([
            String::new(),
            "## Claim Limitations".to_string(),
            String::new(),
        ]);
        lines.extend(report.limitations.iter().map(|item| format!("- {item}")));
    }
    lines.push(String::new());
    lines.join("\n")
}

fn summarize_subject(
    subject_id: &str,
    role: &str,
    samples: &[&SampleRecord],
    bootstrap_resamples: usize,
    seed: u64,
) -> Result<SubjectReport, String> {
    let successful = samples
        .iter()
        .copied()
        .filter(|sample| sample.success)
        .collect::<Vec<_>>();
    let failure_count = samples.len() - successful.len();
    Ok(SubjectReport {
        subject_id: subject_id.to_string(),
        role: role.to_string(),
        measured_sample_count: samples.len(),
        failure_count,
        wall_time_ns: summarize_optional(
            successful.iter().map(|sample| Some(sample.wall_time_ns)),
            bootstrap_resamples,
            seed,
        )?,
        cpu_time_ns: summarize_optional(
            successful.iter().map(|sample| {
                sample
                    .cpu_user_ns
                    .zip(sample.cpu_system_ns)
                    .map(|(user, system)| user.saturating_add(system))
            }),
            bootstrap_resamples,
            seed ^ 0x11,
        )?,
        peak_rss_bytes: summarize_optional(
            successful.iter().map(|sample| sample.peak_rss_bytes),
            bootstrap_resamples,
            seed ^ 0x22,
        )?,
        io_read_bytes: summarize_optional(
            successful.iter().map(|sample| sample.io_read_bytes),
            bootstrap_resamples,
            seed ^ 0x33,
        )?,
        io_write_bytes: summarize_optional(
            successful.iter().map(|sample| sample.io_write_bytes),
            bootstrap_resamples,
            seed ^ 0x44,
        )?,
        transferred_bytes: summarize_optional(
            successful.iter().map(|sample| sample.transferred_bytes),
            bootstrap_resamples,
            seed ^ 0x55,
        )?,
        server_latency_ns: summarize_optional(
            successful.iter().map(|sample| sample.server_latency_ns),
            bootstrap_resamples,
            seed ^ 0x66,
        )?,
        server_health_failure_count: successful
            .iter()
            .filter(|sample| sample.server_health_ok == Some(false))
            .count(),
    })
}

fn summarize_optional(
    samples: impl Iterator<Item = Option<u64>>,
    bootstrap_resamples: usize,
    seed: u64,
) -> Result<Option<DistributionSummary>, String> {
    let values = samples
        .flatten()
        .map(|value| value as f64)
        .collect::<Vec<_>>();
    if values.is_empty() {
        Ok(None)
    } else {
        summarize_samples(&values, bootstrap_resamples, seed).map(Some)
    }
}

fn comparison_for_subjects(subjects: &[SubjectReport]) -> Option<CellComparison> {
    let ait = subjects.iter().find(|subject| subject.role == "ait")?;
    let git = subjects.iter().find(|subject| subject.role == "git")?;
    Some(CellComparison {
        candidate_subject_id: ait.subject_id.clone(),
        baseline_subject_id: git.subject_id.clone(),
        p50_wall_time_ratio: ratio(
            ait.wall_time_ns.as_ref().map(|value| value.p50),
            git.wall_time_ns.as_ref().map(|value| value.p50),
        ),
        p95_wall_time_ratio: ratio(
            ait.wall_time_ns.as_ref().map(|value| value.p95),
            git.wall_time_ns.as_ref().map(|value| value.p95),
        ),
    })
}

fn ratio(candidate: Option<f64>, baseline: Option<f64>) -> Option<f64> {
    candidate
        .zip(baseline)
        .and_then(|(candidate, baseline)| (baseline > 0.0).then_some(candidate / baseline))
}

fn read_raw_records(path: &Path) -> Result<(RawHeader, Vec<SampleRecord>, RawFooter), String> {
    let file = File::open(path)
        .map_err(|error| format!("Failed to open raw benchmark {}: {error}", path.display()))?;
    let mut header = None;
    let mut footer = None;
    let mut samples = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|error| format!("Failed to read raw JSONL: {error}"))?;
        let record = serde_json::from_str::<RawRecord>(&line)
            .map_err(|error| format!("Invalid raw JSONL line {}: {error}", index + 1))?;
        match record {
            RawRecord::Header(value) => {
                if header.replace(value).is_some() || index != 0 {
                    return Err("Raw JSONL must contain exactly one first-line header".to_string());
                }
            }
            RawRecord::Sample(value) => {
                if header.is_none() || footer.is_some() {
                    return Err("Raw sample appears outside header/footer boundaries".to_string());
                }
                samples.push(value);
            }
            RawRecord::Footer(value) => {
                if footer.replace(value).is_some() {
                    return Err("Raw JSONL contains more than one footer".to_string());
                }
            }
        }
    }
    Ok((
        header.ok_or_else(|| "Raw JSONL header is missing".to_string())?,
        samples,
        footer.ok_or_else(|| "Raw JSONL footer is missing".to_string())?,
    ))
}

fn stable_seed(cell_index: usize, subject_index: usize, cell_id: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in cell_id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash ^ ((cell_index as u64) << 32) ^ subject_index as u64
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

fn format_ci_ms(value: Option<[f64; 2]>) -> String {
    value
        .map(|value| {
            format!(
                "{:.3}–{:.3}",
                value[0] / 1_000_000.0,
                value[1] / 1_000_000.0
            )
        })
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

    use crate::model::{
        CellSpec, CommandSpec, EnvironmentPin, FixtureDeclaration, FixtureScale, RawRecord,
        SampleClass, SamplingPolicy, SubjectSpec, Temperature,
    };
    use crate::{MANIFEST_CONTRACT, RAW_CONTRACT};

    use super::*;

    #[test]
    fn ratio_refuses_zero_baseline() {
        assert_eq!(ratio(Some(5.0), Some(2.0)), Some(2.5));
        assert_eq!(ratio(Some(5.0), Some(0.0)), None);
    }

    #[test]
    fn report_keeps_failures_out_of_percentiles_and_claims() {
        let manifest = minimal_manifest();
        let raw = tempfile::NamedTempFile::new().unwrap();
        let header = RawRecord::Header(RawHeader {
            contract: RAW_CONTRACT.to_string(),
            benchmark_id: manifest.benchmark_id.clone(),
            protocol_revision: manifest.protocol_revision.clone(),
            manifest_digest: "sha256:test".to_string(),
            started_at: "2026-07-19T00:00:00Z".to_string(),
            evidence_class: "smoke".to_string(),
            protocol_conformant: false,
            campaign_scope: manifest.campaign_scope.as_str().to_string(),
            seed: 7,
            environment: manifest.environment.clone(),
        });
        let mut records = vec![header];
        records.push(RawRecord::Sample(sample("ait", "ait", 10, true)));
        records.push(RawRecord::Sample(sample("ait", "ait", 20, true)));
        records.push(RawRecord::Sample(sample("git", "git", 5, true)));
        records.push(RawRecord::Sample(sample("git", "git", 999, false)));
        records.push(RawRecord::Footer(RawFooter {
            contract: RAW_CONTRACT.to_string(),
            benchmark_id: manifest.benchmark_id.clone(),
            finished_at: "2026-07-19T00:01:00Z".to_string(),
            sample_count: 4,
            measured_sample_count: 4,
            failure_count: 1,
        }));
        let body = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(raw.path(), body).unwrap();

        let report = build_report(&manifest, "sha256:test", raw.path()).unwrap();
        assert_eq!(report.total_failure_count, 1);
        assert!(!report.claim_eligible);
        let git = report.cells[0]
            .subjects
            .iter()
            .find(|subject| subject.role == "git")
            .unwrap();
        assert_eq!(git.failure_count, 1);
        assert_eq!(git.wall_time_ns.as_ref().unwrap().p50, 5.0);
        assert_eq!(
            report.cells[0]
                .ait_vs_git
                .as_ref()
                .unwrap()
                .p50_wall_time_ratio,
            Some(3.0)
        );
        assert!(render_markdown(&report).contains("Claim eligible: `false`"));
    }

    fn minimal_manifest() -> BenchmarkManifest {
        let command = CommandSpec {
            program: "/usr/bin/true".to_string(),
            args: vec![],
            cwd: None,
            env: BTreeMap::new(),
            expected_exit_codes: vec![0],
        };
        let subject = |role: &str| SubjectSpec {
            subject_id: role.to_string(),
            role: role.to_string(),
            workspace_root: std::path::PathBuf::from("/fixture"),
            metadata_excludes: vec![".ait".to_string(), ".git".to_string()],
            command: command.clone(),
            reset_commands: vec![],
            prepare_commands: vec![],
            cleanup_commands: vec![],
            history_node_probe: command.clone(),
            outcome_probe: command.clone(),
            metrics_json_path: None,
        };
        BenchmarkManifest {
            contract: MANIFEST_CONTRACT.to_string(),
            benchmark_id: "report-test".to_string(),
            protocol_revision: "v1".to_string(),
            campaign_scope: crate::model::CampaignScope::FocusedSlice,
            seed: 7,
            sampling: SamplingPolicy {
                warmup_iterations: 5,
                measured_local_iterations: 50,
                measured_cold_iterations: 30,
            },
            environment: EnvironmentPin {
                captured_at: "2026-07-19T00:00:00Z".to_string(),
                os: "test".to_string(),
                architecture: "test".to_string(),
                filesystem: "test".to_string(),
                storage_medium: "test".to_string(),
                cpu: "test".to_string(),
                memory_bytes: 1,
                rust_version: "test".to_string(),
                git_version: "test".to_string(),
                ait_version: "test".to_string(),
                repository_snapshot: "test".to_string(),
                server_revision: "test".to_string(),
                network_profile: "test".to_string(),
                cache_drop_method: "test".to_string(),
                command_options: BTreeMap::new(),
            },
            fixtures: vec![FixtureDeclaration {
                fixture_id: "small".to_string(),
                revision: "1".to_string(),
                scale: FixtureScale::Small,
                kind: "synthetic".to_string(),
                source: "test".to_string(),
                redistribution: "test".to_string(),
                content_digest: format!("sha256:{}", "a".repeat(64)),
                file_count: 1,
                total_bytes: 1,
                history_nodes: 1,
                features: vec![],
            }],
            cells: vec![CellSpec {
                cell_id: "small-status".to_string(),
                fixture_id: "small".to_string(),
                operation: "status_clean".to_string(),
                temperature: Temperature::Warm,
                sample_class: SampleClass::Local,
                subjects: vec![subject("ait"), subject("git")],
            }],
            bootstrap_resamples: 1_000,
            limitations: vec!["smoke".to_string()],
        }
    }

    fn sample(subject_id: &str, role: &str, wall_time_ns: u64, success: bool) -> SampleRecord {
        SampleRecord {
            contract: RAW_CONTRACT.to_string(),
            benchmark_id: "report-test".to_string(),
            protocol_revision: "v1".to_string(),
            cell_id: "small-status".to_string(),
            fixture_id: "small".to_string(),
            fixture_scale: "small".to_string(),
            fixture_content_digest: format!("sha256:{}", "a".repeat(64)),
            operation: "status_clean".to_string(),
            temperature: "warm".to_string(),
            sample_class: "local".to_string(),
            subject_id: subject_id.to_string(),
            subject_role: role.to_string(),
            block_index: 0,
            randomized_order: 0,
            warmup: false,
            started_at: "2026-07-19T00:00:00Z".to_string(),
            success,
            exit_code: Some(if success { 0 } else { 7 }),
            wall_time_ns,
            cpu_user_ns: Some(wall_time_ns / 2),
            cpu_system_ns: Some(wall_time_ns / 4),
            peak_rss_bytes: Some(1024),
            io_read_bytes: Some(0),
            io_write_bytes: Some(0),
            transferred_bytes: None,
            server_latency_ns: None,
            server_health_ok: None,
            outcome_digest: Some(format!("sha256:{}", "c".repeat(64))),
            failure: (!success).then(|| "expected failure".to_string()),
        }
    }
}
