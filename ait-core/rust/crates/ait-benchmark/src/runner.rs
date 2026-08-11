use std::fs::{self, File};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::fixture::{digest_workspace, profile_workspace};
use crate::model::{
    BenchmarkManifest, CellSpec, CommandSpec, RawFooter, RawHeader, RawRecord, SampleClass,
    SampleRecord, SidecarMetrics, SubjectSpec,
};
use crate::protocol::fixture_for_cell;
use crate::statistics::DeterministicRng;
use crate::RAW_CONTRACT;

#[derive(Clone, Copy, Debug, Default)]
pub struct RunOptions {
    pub smoke: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunSummary {
    pub contract: &'static str,
    pub benchmark_id: String,
    pub raw_jsonl_path: String,
    pub evidence_class: String,
    pub protocol_conformant: bool,
    pub campaign_scope: String,
    pub sample_count: usize,
    pub measured_sample_count: usize,
    pub failure_count: usize,
}

pub fn run_benchmark(
    manifest: &BenchmarkManifest,
    manifest_digest: &str,
    raw_jsonl_path: &Path,
    options: RunOptions,
) -> Result<RunSummary, String> {
    if let Some(parent) = raw_jsonl_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create raw benchmark output directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let file = File::create(raw_jsonl_path).map_err(|error| {
        format!(
            "Failed to create raw benchmark JSONL {}: {error}",
            raw_jsonl_path.display()
        )
    })?;
    let mut writer = BufWriter::new(file);
    let evidence_class = if options.smoke {
        "smoke"
    } else if manifest.campaign_scope.claim_eligible() {
        "measured"
    } else {
        "focused_measured"
    };
    let protocol_conformant = !options.smoke && manifest.campaign_scope.claim_eligible();
    write_record(
        &mut writer,
        &RawRecord::Header(RawHeader {
            contract: RAW_CONTRACT.to_string(),
            benchmark_id: manifest.benchmark_id.clone(),
            protocol_revision: manifest.protocol_revision.clone(),
            manifest_digest: manifest_digest.to_string(),
            started_at: Utc::now().to_rfc3339(),
            evidence_class: evidence_class.to_string(),
            protocol_conformant,
            campaign_scope: manifest.campaign_scope.as_str().to_string(),
            seed: manifest.seed,
            environment: manifest.environment.clone(),
        }),
    )?;

    let warmups = if options.smoke {
        1
    } else {
        manifest.sampling.warmup_iterations
    };
    let mut sample_count = 0_usize;
    let mut measured_sample_count = 0_usize;
    let mut failure_count = 0_usize;
    let mut rng = DeterministicRng::new(manifest.seed);

    for cell in &manifest.cells {
        let fixture = fixture_for_cell(manifest, cell)?;
        prepare_and_verify_cell(
            cell,
            &fixture.content_digest,
            fixture.file_count,
            fixture.total_bytes,
            fixture.history_nodes,
        )?;
        let measured = if options.smoke {
            2
        } else {
            match cell.sample_class {
                SampleClass::Local => manifest.sampling.measured_local_iterations,
                SampleClass::ProcessNetwork => manifest.sampling.measured_cold_iterations,
            }
        };
        for block_index in 0..(warmups + measured) {
            let warmup = block_index < warmups;
            let mut subject_order = (0..cell.subjects.len()).collect::<Vec<_>>();
            rng.shuffle(&mut subject_order);
            let mut block_records = Vec::with_capacity(subject_order.len());
            for (randomized_order, subject_index) in subject_order.into_iter().enumerate() {
                let subject = &cell.subjects[subject_index];
                let record = execute_sample(
                    manifest,
                    cell,
                    fixture,
                    subject,
                    block_index,
                    randomized_order,
                    warmup,
                );
                block_records.push(record);
            }
            enforce_equivalent_outcomes(&mut block_records);
            for record in block_records {
                sample_count += 1;
                if !warmup {
                    measured_sample_count += 1;
                }
                if !record.success {
                    failure_count += 1;
                }
                write_record(&mut writer, &RawRecord::Sample(record))?;
                writer
                    .flush()
                    .map_err(|error| format!("Failed to flush raw benchmark sample: {error}"))?;
            }
        }
    }

    write_record(
        &mut writer,
        &RawRecord::Footer(RawFooter {
            contract: RAW_CONTRACT.to_string(),
            benchmark_id: manifest.benchmark_id.clone(),
            finished_at: Utc::now().to_rfc3339(),
            sample_count,
            measured_sample_count,
            failure_count,
        }),
    )?;
    writer
        .flush()
        .map_err(|error| format!("Failed to finalize raw benchmark JSONL: {error}"))?;
    Ok(RunSummary {
        contract: RAW_CONTRACT,
        benchmark_id: manifest.benchmark_id.clone(),
        raw_jsonl_path: raw_jsonl_path.display().to_string(),
        evidence_class: evidence_class.to_string(),
        protocol_conformant,
        campaign_scope: manifest.campaign_scope.as_str().to_string(),
        sample_count,
        measured_sample_count,
        failure_count,
    })
}

fn prepare_and_verify_cell(
    cell: &CellSpec,
    expected_digest: &str,
    expected_file_count: u64,
    expected_total_bytes: u64,
    expected_history_nodes: u64,
) -> Result<(), String> {
    for subject in &cell.subjects {
        for command in &subject.reset_commands {
            execute_lifecycle_command(command, cell, subject, 0)?;
        }
        let actual = digest_workspace(&subject.workspace_root, &subject.metadata_excludes)?;
        if actual != expected_digest {
            return Err(format!(
                "Cell {} subject {} is not byte-equivalent to fixture {}: expected {}, got {}",
                cell.cell_id, subject.subject_id, cell.fixture_id, expected_digest, actual
            ));
        }
        let (file_count, total_bytes) =
            profile_workspace(&subject.workspace_root, &subject.metadata_excludes)?;
        if file_count != expected_file_count || total_bytes != expected_total_bytes {
            return Err(format!(
                "Cell {} subject {} fixture profile mismatch: expected {expected_file_count} files/{expected_total_bytes} bytes, got {file_count}/{total_bytes}",
                cell.cell_id, subject.subject_id
            ));
        }
        let history_nodes = execute_text_command(&subject.history_node_probe, cell, subject, 0)?
            .parse::<u64>()
            .map_err(|error| {
                format!(
                    "Cell {} subject {} history probe must print one integer: {error}",
                    cell.cell_id, subject.subject_id
                )
            })?;
        if history_nodes != expected_history_nodes {
            return Err(format!(
                "Cell {} subject {} history mismatch: expected {expected_history_nodes} nodes, got {history_nodes}",
                cell.cell_id, subject.subject_id
            ));
        }
    }
    Ok(())
}

fn execute_sample(
    manifest: &BenchmarkManifest,
    cell: &CellSpec,
    fixture: &crate::model::FixtureDeclaration,
    subject: &SubjectSpec,
    block_index: usize,
    randomized_order: usize,
    warmup: bool,
) -> SampleRecord {
    let started_at = Utc::now().to_rfc3339();
    let mut failure = None;
    for command in &subject.prepare_commands {
        if let Err(error) = execute_lifecycle_command(command, cell, subject, block_index) {
            failure = Some(format!("prepare_failed: {error}"));
            break;
        }
    }

    let metrics_path = subject
        .metrics_json_path
        .as_ref()
        .map(|path| expand_path(path, cell, subject, block_index));
    if failure.is_none() {
        if let Some(path) = metrics_path.as_ref() {
            if path.is_file() {
                if let Err(error) = fs::remove_file(path) {
                    failure = Some(format!(
                        "metrics_reset_failed for {}: {error}",
                        path.display()
                    ));
                }
            }
        }
    }

    let measurement = if failure.is_none() {
        match execute_timed_command(&subject.command, cell, subject, block_index) {
            Ok(measurement) => Some(measurement),
            Err(error) => {
                failure = Some(format!("command_failed: {error}"));
                None
            }
        }
    } else {
        None
    };

    let sidecar = if measurement.as_ref().is_some_and(|value| value.success) {
        metrics_path
            .as_deref()
            .map(load_sidecar_metrics)
            .transpose()
            .unwrap_or_else(|error| {
                failure = Some(error);
                None
            })
            .unwrap_or_default()
    } else {
        SidecarMetrics::default()
    };

    let outcome_digest =
        if measurement.as_ref().is_some_and(|value| value.success) && failure.is_none() {
            match execute_text_command(&subject.outcome_probe, cell, subject, block_index) {
                Ok(outcome) => Some(format!("sha256:{:x}", Sha256::digest(outcome.as_bytes()))),
                Err(error) => {
                    failure = Some(format!("outcome_probe_failed: {error}"));
                    None
                }
            }
        } else {
            None
        };

    for command in &subject.cleanup_commands {
        if let Err(error) = execute_lifecycle_command(command, cell, subject, block_index) {
            if failure.is_none() {
                failure = Some(format!("cleanup_failed: {error}"));
            }
        }
    }

    let measurement = measurement.unwrap_or_default();
    let success = measurement.success && failure.is_none();
    if !measurement.success && failure.is_none() {
        failure = measurement.failure.clone();
    }
    SampleRecord {
        contract: RAW_CONTRACT.to_string(),
        benchmark_id: manifest.benchmark_id.clone(),
        protocol_revision: manifest.protocol_revision.clone(),
        cell_id: cell.cell_id.clone(),
        fixture_id: fixture.fixture_id.clone(),
        fixture_scale: fixture.scale.as_str().to_string(),
        fixture_content_digest: fixture.content_digest.clone(),
        operation: cell.operation.clone(),
        temperature: cell.temperature.as_str().to_string(),
        sample_class: cell.sample_class.as_str().to_string(),
        subject_id: subject.subject_id.clone(),
        subject_role: subject.role.clone(),
        block_index,
        randomized_order,
        warmup,
        started_at,
        success,
        exit_code: measurement.exit_code,
        wall_time_ns: measurement.wall_time_ns,
        cpu_user_ns: measurement.cpu_user_ns,
        cpu_system_ns: measurement.cpu_system_ns,
        peak_rss_bytes: measurement.peak_rss_bytes,
        io_read_bytes: measurement.io_read_bytes,
        io_write_bytes: measurement.io_write_bytes,
        transferred_bytes: sidecar.transferred_bytes,
        server_latency_ns: sidecar.server_latency_ns,
        server_health_ok: sidecar.server_health_ok,
        outcome_digest,
        failure,
    }
}

fn enforce_equivalent_outcomes(records: &mut [SampleRecord]) {
    if records.iter().any(|record| !record.success) {
        return;
    }
    let expected = records
        .first()
        .and_then(|record| record.outcome_digest.clone());
    if expected.is_some()
        && records
            .iter()
            .all(|record| record.outcome_digest == expected)
    {
        return;
    }
    let evidence = records
        .iter()
        .map(|record| {
            format!(
                "{}={}",
                record.subject_id,
                record.outcome_digest.as_deref().unwrap_or("missing")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    for record in records {
        record.success = false;
        record.failure = Some(format!("outcome_mismatch: {evidence}"));
    }
}

fn execute_lifecycle_command(
    spec: &CommandSpec,
    cell: &CellSpec,
    subject: &SubjectSpec,
    iteration: usize,
) -> Result<(), String> {
    let mut command = command_from_spec(spec, cell, subject, iteration)?;
    let output = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("Failed to launch {}: {error}", spec.program))?;
    let code = output.status.code();
    if !code.is_some_and(|code| spec.expected_exit_codes.contains(&code)) {
        return Err(format!(
            "{} exited {:?}: {}",
            spec.program,
            code,
            bounded_text(&output.stderr)
        ));
    }
    Ok(())
}

fn execute_text_command(
    spec: &CommandSpec,
    cell: &CellSpec,
    subject: &SubjectSpec,
    iteration: usize,
) -> Result<String, String> {
    let output = command_from_spec(spec, cell, subject, iteration)?
        .output()
        .map_err(|error| format!("Failed to launch {}: {error}", spec.program))?;
    let code = output.status.code();
    if !code.is_some_and(|code| spec.expected_exit_codes.contains(&code)) {
        return Err(format!(
            "{} exited {:?}: {}",
            spec.program,
            code,
            bounded_text(&output.stderr)
        ));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|_| format!("{} probe output is not UTF-8", spec.program))?
        .trim()
        .to_string();
    if value.is_empty() {
        return Err(format!("{} probe output is empty", spec.program));
    }
    Ok(value)
}

#[derive(Default)]
struct ProcessMeasurement {
    success: bool,
    exit_code: Option<i32>,
    wall_time_ns: u64,
    cpu_user_ns: Option<u64>,
    cpu_system_ns: Option<u64>,
    peak_rss_bytes: Option<u64>,
    io_read_bytes: Option<u64>,
    io_write_bytes: Option<u64>,
    failure: Option<String>,
}

#[cfg(unix)]
fn execute_timed_command(
    spec: &CommandSpec,
    cell: &CellSpec,
    subject: &SubjectSpec,
    iteration: usize,
) -> Result<ProcessMeasurement, String> {
    let mut stdout = tempfile::tempfile()
        .map_err(|error| format!("Failed to create benchmark stdout capture: {error}"))?;
    let mut stderr = tempfile::tempfile()
        .map_err(|error| format!("Failed to create benchmark stderr capture: {error}"))?;
    let mut command = command_from_spec(spec, cell, subject, iteration)?;
    command.stdout(Stdio::from(
        stdout
            .try_clone()
            .map_err(|error| format!("Failed to clone stdout capture: {error}"))?,
    ));
    command.stderr(Stdio::from(
        stderr
            .try_clone()
            .map_err(|error| format!("Failed to clone stderr capture: {error}"))?,
    ));
    let started = Instant::now();
    let child = command
        .spawn()
        .map_err(|error| format!("Failed to launch {}: {error}", spec.program))?;
    let pid = child.id() as libc::pid_t;
    let mut status = 0_i32;
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    let waited = unsafe { libc::wait4(pid, &mut status, 0, usage.as_mut_ptr()) };
    let wall_time_ns = saturating_nanos(started.elapsed().as_nanos());
    drop(child);
    if waited < 0 {
        return Err(format!(
            "wait4 failed for {}: {}",
            spec.program,
            std::io::Error::last_os_error()
        ));
    }
    let usage = unsafe { usage.assume_init() };
    let exit_code = if libc::WIFEXITED(status) {
        Some(libc::WEXITSTATUS(status))
    } else if libc::WIFSIGNALED(status) {
        Some(-libc::WTERMSIG(status))
    } else {
        None
    };
    let success = exit_code.is_some_and(|code| spec.expected_exit_codes.contains(&code));
    let failure = if success {
        None
    } else {
        let stderr_text = read_capture(&mut stderr);
        let stdout_text = read_capture(&mut stdout);
        Some(format!(
            "{} exited {:?}; stderr={}; stdout={}",
            spec.program, exit_code, stderr_text, stdout_text
        ))
    };
    Ok(ProcessMeasurement {
        success,
        exit_code,
        wall_time_ns,
        cpu_user_ns: Some(timeval_nanos(usage.ru_utime)),
        cpu_system_ns: Some(timeval_nanos(usage.ru_stime)),
        peak_rss_bytes: Some(max_rss_bytes(usage.ru_maxrss)),
        io_read_bytes: nonnegative_u64(usage.ru_inblock).map(|blocks| blocks.saturating_mul(512)),
        io_write_bytes: nonnegative_u64(usage.ru_oublock).map(|blocks| blocks.saturating_mul(512)),
        failure,
    })
}

#[cfg(not(unix))]
fn execute_timed_command(
    spec: &CommandSpec,
    cell: &CellSpec,
    subject: &SubjectSpec,
    iteration: usize,
) -> Result<ProcessMeasurement, String> {
    let started = Instant::now();
    let output = command_from_spec(spec, cell, subject, iteration)?
        .output()
        .map_err(|error| format!("Failed to launch {}: {error}", spec.program))?;
    let exit_code = output.status.code();
    let success = exit_code.is_some_and(|code| spec.expected_exit_codes.contains(&code));
    Ok(ProcessMeasurement {
        success,
        exit_code,
        wall_time_ns: saturating_nanos(started.elapsed().as_nanos()),
        failure: (!success).then(|| {
            format!(
                "{} exited {:?}: {}",
                spec.program,
                exit_code,
                bounded_text(&output.stderr)
            )
        }),
        ..ProcessMeasurement::default()
    })
}

fn command_from_spec(
    spec: &CommandSpec,
    cell: &CellSpec,
    subject: &SubjectSpec,
    iteration: usize,
) -> Result<Command, String> {
    let program = expand(&spec.program, cell, subject, iteration);
    if program.trim().is_empty() {
        return Err("Expanded benchmark program is empty".to_string());
    }
    let mut command = Command::new(program);
    command.args(
        spec.args
            .iter()
            .map(|arg| expand(arg, cell, subject, iteration)),
    );
    let cwd = spec
        .cwd
        .as_ref()
        .map(|path| expand_path(path, cell, subject, iteration))
        .unwrap_or_else(|| subject.workspace_root.clone());
    command.current_dir(cwd);
    for (key, value) in &spec.env {
        command.env(key, expand(value, cell, subject, iteration));
    }
    Ok(command)
}

fn expand(value: &str, cell: &CellSpec, subject: &SubjectSpec, iteration: usize) -> String {
    value
        .replace("{workspace}", &subject.workspace_root.display().to_string())
        .replace("{cell}", &cell.cell_id)
        .replace("{subject}", &subject.subject_id)
        .replace("{iteration}", &iteration.to_string())
}

fn expand_path(path: &Path, cell: &CellSpec, subject: &SubjectSpec, iteration: usize) -> PathBuf {
    PathBuf::from(expand(&path.to_string_lossy(), cell, subject, iteration))
}

fn load_sidecar_metrics(path: &Path) -> Result<SidecarMetrics, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "metrics sidecar {} was not produced or readable: {error}",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("metrics sidecar {} is invalid: {error}", path.display()))
}

fn write_record(writer: &mut impl Write, record: &RawRecord) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, record)
        .map_err(|error| format!("Failed to encode raw benchmark record: {error}"))?;
    writer
        .write_all(b"\n")
        .map_err(|error| format!("Failed to write raw benchmark record: {error}"))
}

fn bounded_text(bytes: &[u8]) -> String {
    let bounded = &bytes[..bytes.len().min(4_096)];
    String::from_utf8_lossy(bounded).replace(['\n', '\r'], " ")
}

#[cfg(unix)]
fn read_capture(file: &mut File) -> String {
    let _ = file.seek(SeekFrom::Start(0));
    let mut bytes = Vec::new();
    let _ = file.take(4_096).read_to_end(&mut bytes);
    bounded_text(&bytes)
}

fn saturating_nanos(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(unix)]
fn timeval_nanos(value: libc::timeval) -> u64 {
    let seconds = nonnegative_u64(value.tv_sec).unwrap_or(0);
    let micros = nonnegative_u64(value.tv_usec).unwrap_or(0);
    seconds
        .saturating_mul(1_000_000_000)
        .saturating_add(micros.saturating_mul(1_000))
}

#[cfg(all(unix, target_os = "linux"))]
fn max_rss_bytes(value: libc::c_long) -> u64 {
    nonnegative_u64(value).unwrap_or(0).saturating_mul(1024)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn max_rss_bytes(value: libc::c_long) -> u64 {
    nonnegative_u64(value).unwrap_or(0)
}

#[cfg(unix)]
fn nonnegative_u64<T>(value: T) -> Option<u64>
where
    T: TryInto<i128>,
{
    let value = value.try_into().ok()?;
    (value >= 0).then(|| u64::try_from(value).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::model::{CommandSpec, SampleClass, SubjectSpec, Temperature};

    fn fixture_subject(root: &Path) -> (CellSpec, SubjectSpec) {
        let command = CommandSpec {
            program: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "test -d {workspace}".to_string()],
            cwd: None,
            env: BTreeMap::new(),
            expected_exit_codes: vec![0],
        };
        let subject = SubjectSpec {
            subject_id: "ait".to_string(),
            role: "ait".to_string(),
            workspace_root: root.to_path_buf(),
            metadata_excludes: vec![".ait".to_string(), ".git".to_string()],
            command,
            reset_commands: vec![],
            prepare_commands: vec![],
            cleanup_commands: vec![],
            history_node_probe: CommandSpec {
                program: "/bin/echo".to_string(),
                args: vec!["1".to_string()],
                cwd: None,
                env: BTreeMap::new(),
                expected_exit_codes: vec![0],
            },
            outcome_probe: CommandSpec {
                program: "/bin/echo".to_string(),
                args: vec!["same".to_string()],
                cwd: None,
                env: BTreeMap::new(),
                expected_exit_codes: vec![0],
            },
            metrics_json_path: None,
        };
        let cell = CellSpec {
            cell_id: "small-status".to_string(),
            fixture_id: "small-v1".to_string(),
            operation: "status_clean".to_string(),
            temperature: Temperature::Warm,
            sample_class: SampleClass::Local,
            subjects: vec![subject.clone()],
        };
        (cell, subject)
    }

    #[test]
    fn timed_external_subject_records_resource_and_exit_contract() {
        let root = tempfile::tempdir().unwrap();
        let (cell, subject) = fixture_subject(root.path());
        let measurement = execute_timed_command(&subject.command, &cell, &subject, 3).unwrap();
        assert!(measurement.success);
        assert_eq!(measurement.exit_code, Some(0));
        assert!(measurement.wall_time_ns > 0);
        #[cfg(unix)]
        assert!(measurement.cpu_user_ns.is_some());
    }

    #[test]
    fn unexpected_exit_is_retained_as_failure_evidence() {
        let root = tempfile::tempdir().unwrap();
        let (cell, mut subject) = fixture_subject(root.path());
        subject.command.args = vec!["-c".to_string(), "exit 7".to_string()];
        let measurement = execute_timed_command(&subject.command, &cell, &subject, 0).unwrap();
        assert!(!measurement.success);
        assert_eq!(measurement.exit_code, Some(7));
        assert!(measurement.failure.unwrap().contains("exited Some(7)"));
    }

    #[test]
    fn mismatched_outcomes_fail_every_subject_in_the_block() {
        let mut records = vec![
            SampleRecord {
                outcome_digest: Some("sha256:one".to_string()),
                success: true,
                subject_id: "ait".to_string(),
                ..sample_record_fixture()
            },
            SampleRecord {
                outcome_digest: Some("sha256:two".to_string()),
                success: true,
                subject_id: "git".to_string(),
                ..sample_record_fixture()
            },
        ];
        enforce_equivalent_outcomes(&mut records);
        assert!(records.iter().all(|record| !record.success));
        assert!(records.iter().all(|record| record
            .failure
            .as_deref()
            .unwrap()
            .contains("outcome_mismatch")));
    }

    fn sample_record_fixture() -> SampleRecord {
        SampleRecord {
            contract: RAW_CONTRACT.to_string(),
            benchmark_id: "test".to_string(),
            protocol_revision: "v1".to_string(),
            cell_id: "cell".to_string(),
            fixture_id: "fixture".to_string(),
            fixture_scale: "small".to_string(),
            fixture_content_digest: "sha256:test".to_string(),
            operation: "status_clean".to_string(),
            temperature: "warm".to_string(),
            sample_class: "local".to_string(),
            subject_id: "subject".to_string(),
            subject_role: "ait".to_string(),
            block_index: 0,
            randomized_order: 0,
            warmup: false,
            started_at: "2026-07-19T00:00:00Z".to_string(),
            success: true,
            exit_code: Some(0),
            wall_time_ns: 1,
            cpu_user_ns: Some(1),
            cpu_system_ns: Some(0),
            peak_rss_bytes: Some(1),
            io_read_bytes: Some(0),
            io_write_bytes: Some(0),
            transferred_bytes: None,
            server_latency_ns: None,
            server_health_ok: None,
            outcome_digest: Some("sha256:same".to_string()),
            failure: None,
        }
    }
}
