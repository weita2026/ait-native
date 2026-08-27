use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use ait_benchmark::{
    build_agent_token_schedule, build_report, compare_agent_token_reports, compare_reports,
    create_synthetic_fixture, digest_workspace, encode_manifest,
    extract_and_validate_codex_transcript, import_codex_usage, load_agent_token_campaign,
    load_agent_token_campaign_for_evidence, load_agent_token_campaign_statistical_view,
    load_agent_token_report, load_agent_token_schedule, load_benchmark_report,
    load_budget_manifest, load_manifest, materialize_game_fixture, normalize_manifest,
    render_agent_token_report_markdown, resume_agent_token_campaign, run_agent_token_campaign,
    run_agent_token_statistical_replacement, run_benchmark, sha256_digest,
    validate_agent_token_campaign_evidence, validate_manifest, validate_portable_manifest,
    write_agent_token_publication_bundle, write_comparison_report, write_json_new, write_report,
    write_text_new, AgentTokenAccountingProfile, AgentTokenMode, AgentTokenModelPin,
    AgentTokenPublicationInput, NormalizationReport, RunOptions, RuntimeBindings,
    SyntheticFixtureRecipe, AGENT_TOKEN_PROTOCOL_V1_JSON, NORMALIZATION_CONTRACT, PROTOCOL_V1_JSON,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(
    name = "ait-benchmark",
    version,
    about = "Run the Python-free, versioned AIT/Git benchmark protocols"
)]
struct Cli {
    #[command(subcommand)]
    command: BenchmarkCommand,
}

#[derive(Debug, Subcommand)]
enum BenchmarkCommand {
    /// Print the compiled versioned VCS benchmark protocol contract.
    Protocol,
    /// Validate the full multi-scale benchmark manifest and pinned environment.
    Validate(ManifestArgs),
    /// Replace explicit runtime paths with portable binding placeholders.
    Normalize(NormalizeArgs),
    /// Execute external AIT and Git subjects and write authoritative raw JSONL.
    Run(RunArgs),
    /// Render JSON and Markdown statistics from authoritative raw JSONL.
    Report(ReportArgs),
    /// Compare baseline and candidate reports against a versioned performance budget.
    Compare(CompareArgs),
    /// Create or digest byte-equivalent benchmark fixtures.
    Fixture {
        #[command(subcommand)]
        command: FixtureCommand,
    },
    /// Run canonical benchmark probes used by subject history/equivalence gates.
    Probe {
        #[command(subcommand)]
        command: ProbeCommand,
    },
    /// Measure fresh coding-agent token usage for deterministic game-development tasks.
    AgentToken {
        #[command(subcommand)]
        command: AgentTokenCommand,
    },
}

#[derive(Debug, Subcommand)]
enum AgentTokenCommand {
    /// Print the compiled game-development agent-token protocol.
    Protocol,
    /// Materialize one immutable game workload into an absent or empty directory.
    Fixture(AgentTokenFixtureArgs),
    /// Validate a campaign manifest and, optionally, its complete evidence directory.
    Validate(AgentTokenValidateArgs),
    /// Freeze the deterministic block-randomized campaign schedule.
    Schedule(AgentTokenScheduleArgs),
    /// Import provider-reported Codex JSONL usage without double-counting cache fields.
    ImportUsage(AgentTokenImportUsageArgs),
    /// Validate and normalize repository command events from a Codex JSONL transcript.
    ImportTranscript(AgentTokenImportTranscriptArgs),
    /// Execute the frozen schedule into a create-new evidence directory.
    Run(AgentTokenRunArgs),
    /// Resume the exact missing suffix of an existing immutable campaign.
    Resume(AgentTokenResumeArgs),
    /// Execute the exact owner-authorized transparent statistical replacement lane.
    Replace(AgentTokenReplaceArgs),
    /// Rebuild aggregate and Markdown reports from immutable per-run summaries.
    Report(AgentTokenReportArgs),
    /// Produce a sanitized, checksummed public bundle from immutable campaign evidence.
    Publish(AgentTokenPublishArgs),
    /// Compare two compatible, already aggregated agent-token campaigns.
    Compare(AgentTokenCompareArgs),
}

#[derive(Debug, Args)]
struct AgentTokenFixtureArgs {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    workload: String,
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long)]
    receipt: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct AgentTokenValidateArgs {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    campaign_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct AgentTokenScheduleArgs {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AgentTokenCliMode {
    #[value(name = "git_linear_single_session")]
    GitLinearSingleSession,
    #[value(name = "ait_linear_single_session")]
    AitLinearSingleSession,
}

impl From<AgentTokenCliMode> for AgentTokenMode {
    fn from(value: AgentTokenCliMode) -> Self {
        match value {
            AgentTokenCliMode::GitLinearSingleSession => Self::GitLinearSingleSession,
            AgentTokenCliMode::AitLinearSingleSession => Self::AitLinearSingleSession,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AgentTokenCliProfile {
    #[value(name = "steady_state_task_cost")]
    SteadyStateTaskCost,
    #[value(name = "first_use_total_cost")]
    FirstUseTotalCost,
}

impl From<AgentTokenCliProfile> for AgentTokenAccountingProfile {
    fn from(value: AgentTokenCliProfile) -> Self {
        match value {
            AgentTokenCliProfile::SteadyStateTaskCost => Self::SteadyStateTaskCost,
            AgentTokenCliProfile::FirstUseTotalCost => Self::FirstUseTotalCost,
        }
    }
}

#[derive(Debug, Args)]
struct AgentTokenImportUsageArgs {
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long)]
    run_id: String,
    #[arg(long)]
    workload: String,
    #[arg(long, value_enum)]
    mode: AgentTokenCliMode,
    #[arg(long, value_enum)]
    profile: AgentTokenCliProfile,
    #[arg(long)]
    model_provider: String,
    #[arg(long)]
    model_id: String,
    #[arg(long)]
    model_revision: String,
    #[arg(long)]
    reasoning_effort: String,
}

#[derive(Debug, Args)]
struct AgentTokenImportTranscriptArgs {
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long)]
    run_id: String,
    #[arg(long, value_enum)]
    mode: AgentTokenCliMode,
    #[arg(long, value_enum)]
    profile: AgentTokenCliProfile,
}

#[derive(Debug, Args)]
struct AgentTokenRunArgs {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    output_dir: PathBuf,
    /// Execute only the first N adjacent workload/attempt pairs.
    #[arg(long)]
    max_pairs: Option<usize>,
}

#[derive(Debug, Args)]
struct AgentTokenResumeArgs {
    #[arg(long)]
    campaign_dir: PathBuf,
    /// Execute only the next N complete adjacent pairs from the missing suffix.
    #[arg(long)]
    max_pairs: Option<usize>,
    /// Append supported validator-only adjudications before checking the prefix.
    #[arg(long)]
    adjudicate_transcripts: bool,
}

#[derive(Debug, Args)]
struct AgentTokenReplaceArgs {
    #[arg(long)]
    campaign_dir: PathBuf,
    #[arg(long)]
    source_run_id: String,
}

#[derive(Debug, Args)]
struct AgentTokenReportArgs {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    campaign_dir: PathBuf,
    #[arg(long)]
    output_json: PathBuf,
    #[arg(long)]
    output_markdown: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct AgentTokenPublishArgs {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    campaign_dir: PathBuf,
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long)]
    release_version: String,
    #[arg(long)]
    measured_product_snapshot: String,
    #[arg(long)]
    measured_ait_sha256: String,
    #[arg(long)]
    campaign_runner_sha256: String,
}

#[derive(Debug, Args)]
struct AgentTokenCompareArgs {
    #[arg(long)]
    baseline_report: PathBuf,
    #[arg(long)]
    candidate_report: PathBuf,
    #[arg(long)]
    output_json: PathBuf,
}

#[derive(Debug, Args)]
struct ManifestArgs {
    #[arg(long)]
    manifest: PathBuf,
    /// Also reject absolute or host-specific paths and validate binding syntax.
    #[arg(long)]
    portable: bool,
}

#[derive(Debug, Args)]
struct NormalizeArgs {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    output: PathBuf,
    /// Runtime path mapping in NAME=ABSOLUTE_PATH form; repeat for each path.
    #[arg(long = "bind", value_name = "NAME=ABSOLUTE_PATH", required = true)]
    bindings: Vec<String>,
}

#[derive(Debug, Args)]
struct RunArgs {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    raw_jsonl: PathBuf,
    /// Run one warm-up and two measured iterations per subject; output is not claim eligible.
    #[arg(long)]
    smoke: bool,
    /// Resolve a portable manifest binding in NAME=ABSOLUTE_PATH form.
    #[arg(long = "bind", value_name = "NAME=ABSOLUTE_PATH")]
    bindings: Vec<String>,
}

#[derive(Debug, Args)]
struct ReportArgs {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    raw_jsonl: PathBuf,
    #[arg(long)]
    output_json: PathBuf,
    #[arg(long)]
    output_markdown: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct CompareArgs {
    #[arg(long)]
    baseline_report: PathBuf,
    #[arg(long)]
    candidate_report: PathBuf,
    #[arg(long)]
    budget: PathBuf,
    #[arg(long)]
    output_json: PathBuf,
    #[arg(long)]
    output_markdown: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum FixtureCommand {
    /// Materialize a deterministic synthetic fixture into an absent or empty directory.
    Create(FixtureCreateArgs),
    /// Calculate the canonical workspace payload digest, excluding VCS metadata.
    Digest(FixtureDigestArgs),
}

#[derive(Debug, Args)]
struct FixtureCreateArgs {
    #[arg(long)]
    recipe: PathBuf,
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long)]
    receipt: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct FixtureDigestArgs {
    #[arg(long)]
    root: PathBuf,
    #[arg(long = "exclude", default_values_t = vec![".ait".to_string(), ".git".to_string()])]
    excludes: Vec<String>,
    /// Print only the canonical digest, suitable for an outcome probe.
    #[arg(long)]
    plain: bool,
}

#[derive(Debug, Subcommand)]
enum ProbeCommand {
    /// Count history nodes using the selected external AIT or Git subject.
    History(HistoryProbeArgs),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ProbeVcs {
    Ait,
    Git,
}

#[derive(Debug, Args)]
struct HistoryProbeArgs {
    #[arg(long, value_enum)]
    vcs: ProbeVcs,
    #[arg(long)]
    program: PathBuf,
    #[arg(long)]
    root: PathBuf,
}

fn main() -> ExitCode {
    match entry(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let diagnostic = serde_json::json!({
                "contract": "ait-vcs-benchmark-diagnostic/v1",
                "status": "error",
                "message": error,
            });
            let _ = writeln!(io::stderr(), "{}", diagnostic);
            ExitCode::from(2)
        }
    }
}

fn entry(cli: Cli) -> Result<(), String> {
    match cli.command {
        BenchmarkCommand::Protocol => {
            let protocol = serde_json::from_str::<serde_json::Value>(PROTOCOL_V1_JSON)
                .map_err(|error| format!("Compiled benchmark protocol is invalid: {error}"))?;
            emit_json(&protocol)
        }
        BenchmarkCommand::Validate(args) => {
            let (manifest, digest) = load_manifest(&args.manifest)?;
            let validation = validate_manifest(&manifest, &digest);
            if args.portable {
                let portability = validate_portable_manifest(&manifest);
                let valid = validation.valid && portability.portable;
                emit_json(&serde_json::json!({
                    "manifest": validation,
                    "portability": portability,
                    "valid": valid,
                }))?;
                if valid {
                    Ok(())
                } else {
                    Err("Benchmark manifest failed protocol or portability validation".to_string())
                }
            } else {
                emit_json(&validation)?;
                if validation.valid {
                    Ok(())
                } else {
                    Err(format!(
                        "Benchmark manifest failed validation with {} error(s)",
                        validation.errors.len()
                    ))
                }
            }
        }
        BenchmarkCommand::Normalize(args) => {
            let bindings = RuntimeBindings::parse(&args.bindings)?;
            let (manifest, source_digest) = load_and_validate(&args.manifest)?;
            let normalized = normalize_manifest(&manifest, &bindings)?;
            let bytes = encode_manifest(&normalized.manifest)?;
            let normalized_digest = sha256_digest(&bytes);
            if let Some(parent) = args
                .output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "Failed to create normalized manifest directory {}: {error}",
                        parent.display()
                    )
                })?;
            }
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&args.output)
                .map_err(|error| {
                    format!(
                        "Failed to create normalized manifest {} without overwriting: {error}",
                        args.output.display()
                    )
                })?;
            output.write_all(&bytes).map_err(|error| {
                format!(
                    "Failed to write normalized manifest {}: {error}",
                    args.output.display()
                )
            })?;
            emit_json(&NormalizationReport {
                contract: NORMALIZATION_CONTRACT,
                benchmark_id: normalized.manifest.benchmark_id,
                source_manifest_digest: source_digest,
                normalized_manifest_digest: normalized_digest,
                output_path: args.output.display().to_string(),
                replacement_count: normalized.replacement_count,
                required_bindings: normalized.required_bindings,
                portable: true,
            })
        }
        BenchmarkCommand::Run(args) => {
            let (manifest, digest) = load_and_validate(&args.manifest)?;
            let bindings = RuntimeBindings::parse(&args.bindings)?;
            let summary = run_benchmark(
                &manifest,
                &digest,
                &args.raw_jsonl,
                RunOptions {
                    smoke: args.smoke,
                    bindings,
                },
            )?;
            emit_json(&summary)
        }
        BenchmarkCommand::Report(args) => {
            let (manifest, digest) = load_and_validate(&args.manifest)?;
            let report = build_report(&manifest, &digest, &args.raw_jsonl)?;
            write_report(&report, &args.output_json, args.output_markdown.as_deref())?;
            emit_json(&serde_json::json!({
                "contract": report.contract,
                "benchmark_id": report.benchmark_id,
                "output_json": args.output_json,
                "output_markdown": args.output_markdown,
                "claim_eligible": report.claim_eligible,
                "failure_count": report.total_failure_count,
            }))
        }
        BenchmarkCommand::Compare(args) => {
            let baseline = load_benchmark_report(&args.baseline_report)?;
            let candidate = load_benchmark_report(&args.candidate_report)?;
            let budget = load_budget_manifest(&args.budget)?;
            let report = compare_reports(&baseline, &candidate, &budget)?;
            write_comparison_report(&report, &args.output_json, args.output_markdown.as_deref())?;
            emit_json(&serde_json::json!({
                "contract": report.contract,
                "budget_id": report.budget_id,
                "output_json": args.output_json,
                "output_markdown": args.output_markdown,
                "budget_passed": report.budget_passed,
                "promotion_ready": report.promotion_ready,
                "blocker_count": report.blockers.len(),
            }))
        }
        BenchmarkCommand::Fixture { command } => match command {
            FixtureCommand::Create(args) => {
                let bytes = fs::read(&args.recipe).map_err(|error| {
                    format!(
                        "Failed to read fixture recipe {}: {error}",
                        args.recipe.display()
                    )
                })?;
                let recipe =
                    serde_json::from_slice::<SyntheticFixtureRecipe>(&bytes).map_err(|error| {
                        format!(
                            "Failed to decode fixture recipe {}: {error}",
                            args.recipe.display()
                        )
                    })?;
                let receipt = create_synthetic_fixture(&recipe, &args.output_dir)?;
                if let Some(path) = args.receipt {
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).map_err(|error| {
                            format!(
                                "Failed to create receipt directory {}: {error}",
                                parent.display()
                            )
                        })?;
                    }
                    let mut output = serde_json::to_vec_pretty(&receipt)
                        .map_err(|error| format!("Failed to encode fixture receipt: {error}"))?;
                    output.push(b'\n');
                    fs::write(&path, output).map_err(|error| {
                        format!(
                            "Failed to write fixture receipt {}: {error}",
                            path.display()
                        )
                    })?;
                }
                emit_json(&receipt)
            }
            FixtureCommand::Digest(args) => {
                let digest = digest_workspace(&args.root, &args.excludes)?;
                if args.plain {
                    writeln!(io::stdout(), "{digest}")
                        .map_err(|error| format!("Failed to write digest: {error}"))
                } else {
                    emit_json(&serde_json::json!({
                        "contract": "ait-vcs-benchmark-fixture-digest/v1",
                        "root": args.root,
                        "excludes": args.excludes,
                        "content_digest": digest,
                    }))
                }
            }
        },
        BenchmarkCommand::Probe { command } => match command {
            ProbeCommand::History(args) => {
                let count = history_node_count(args.vcs, &args.program, &args.root)?;
                writeln!(io::stdout(), "{count}")
                    .map_err(|error| format!("Failed to write history count: {error}"))
            }
        },
        BenchmarkCommand::AgentToken { command } => match command {
            AgentTokenCommand::Protocol => {
                let protocol =
                    serde_json::from_str::<serde_json::Value>(AGENT_TOKEN_PROTOCOL_V1_JSON)
                        .map_err(|error| {
                            format!("Compiled agent-token benchmark protocol is invalid: {error}")
                        })?;
                emit_json(&protocol)
            }
            AgentTokenCommand::Fixture(args) => {
                let receipt =
                    materialize_game_fixture(&args.manifest, &args.workload, &args.output_dir)?;
                if let Some(path) = args.receipt {
                    write_json_new(&path, &receipt)?;
                }
                emit_json(&receipt)
            }
            AgentTokenCommand::Validate(args) => {
                let manifest = if args.campaign_dir.is_some() {
                    load_agent_token_campaign_for_evidence(&args.manifest)?
                } else {
                    load_agent_token_campaign(&args.manifest)?
                };
                let evidence_errors = if let Some(path) = args.campaign_dir.as_deref() {
                    validate_agent_token_campaign_evidence(&manifest, path)?
                } else {
                    Vec::new()
                };
                let valid = evidence_errors.is_empty();
                emit_json(&serde_json::json!({
                    "contract": "ait-agent-token-validation/v1",
                    "campaign_id": manifest.campaign_id,
                    "executor": manifest.runtime.executor.as_str(),
                    "tool_policy": manifest.tool_policy,
                    "manifest_valid": true,
                    "evidence_checked": args.campaign_dir.is_some(),
                    "valid": valid,
                    "errors": evidence_errors,
                    "workflow_mode": "solo_local",
                    "sprint_mode": "off",
                    "ait_server_connected": false,
                }))?;
                if valid {
                    Ok(())
                } else {
                    Err("Agent-token campaign evidence failed validation".to_string())
                }
            }
            AgentTokenCommand::Schedule(args) => {
                let manifest = load_agent_token_campaign(&args.manifest)?;
                let schedule = build_agent_token_schedule(&manifest);
                if let Some(path) = args.output {
                    write_json_new(&path, &schedule)?;
                }
                emit_json(&schedule)
            }
            AgentTokenCommand::ImportUsage(args) => {
                let model = AgentTokenModelPin {
                    provider: args.model_provider,
                    model_id: args.model_id,
                    model_revision: args.model_revision,
                    reasoning_effort: args.reasoning_effort,
                };
                let usage = import_codex_usage(
                    &args.source,
                    &args.run_id,
                    &args.workload,
                    args.mode.into(),
                    args.profile.into(),
                    &model,
                )?;
                if let Some(path) = args.output {
                    write_text_new(
                        &path,
                        &format!(
                            "{}\n",
                            serde_json::to_string(&usage).map_err(|error| {
                                format!("Failed to encode normalized usage JSONL: {error}")
                            })?
                        ),
                    )?;
                }
                emit_json(&usage)
            }
            AgentTokenCommand::ImportTranscript(args) => {
                let transcript = extract_and_validate_codex_transcript(
                    &args.source,
                    &args.run_id,
                    args.mode.into(),
                    args.profile.into(),
                )?;
                if let Some(path) = args.output {
                    write_json_new(&path, &transcript)?;
                }
                let valid = transcript.valid;
                emit_json(&transcript)?;
                if valid {
                    Ok(())
                } else {
                    Err("Agent-token command transcript violates the selected mode".to_string())
                }
            }
            AgentTokenCommand::Run(args) => {
                let execution =
                    run_agent_token_campaign(&args.manifest, &args.output_dir, args.max_pairs)?;
                emit_json(&execution)?;
                if execution.stop_reason.is_some() {
                    Err(
                        "Agent-token campaign stopped before completing its admitted pair slice"
                            .to_string(),
                    )
                } else {
                    Ok(())
                }
            }
            AgentTokenCommand::Resume(args) => {
                let execution = resume_agent_token_campaign(
                    &args.campaign_dir,
                    args.max_pairs,
                    args.adjudicate_transcripts,
                )?;
                let stopped_early = execution.stopped_early;
                emit_json(&execution)?;
                if stopped_early {
                    Err(
                        "Agent-token campaign resume stopped before completing its admitted pair slice"
                            .to_string(),
                    )
                } else {
                    Ok(())
                }
            }
            AgentTokenCommand::Replace(args) => {
                let execution = run_agent_token_statistical_replacement(
                    &args.campaign_dir,
                    &args.source_run_id,
                )?;
                let admitted = execution.selection_activated && execution.claim_eligible;
                emit_json(&execution)?;
                if admitted {
                    Ok(())
                } else {
                    Err(
                        "Agent-token statistical replacement did not satisfy every admission and claim gate"
                            .to_string(),
                    )
                }
            }
            AgentTokenCommand::Report(args) => {
                let manifest = load_agent_token_campaign_for_evidence(&args.manifest)?;
                let schedule = load_agent_token_schedule(
                    &args.campaign_dir.join("randomization-schedule.json"),
                )?;
                let evidence_errors =
                    validate_agent_token_campaign_evidence(&manifest, &args.campaign_dir)?;
                if !evidence_errors.is_empty() {
                    return Err(format!(
                        "Agent-token evidence failed validation: {}",
                        evidence_errors.join("; ")
                    ));
                }
                let statistical_view = load_agent_token_campaign_statistical_view(
                    &manifest,
                    &schedule,
                    &args.campaign_dir,
                )?;
                let report = statistical_view.report;
                write_json_new(&args.output_json, &report)?;
                if let Some(path) = args.output_markdown.as_deref() {
                    write_text_new(path, &render_agent_token_report_markdown(&report))?;
                }
                emit_json(&serde_json::json!({
                    "contract": report.contract,
                    "campaign_id": report.campaign_id,
                    "output_json": args.output_json,
                    "output_markdown": args.output_markdown,
                    "source_protocol_claim_eligible": report.source_protocol_claim_eligible,
                    "current_policy_revision": report.current_policy_revision,
                    "current_policy_evaluation_mode": report.current_policy_evaluation_mode,
                    "current_policy_criteria_met": report.current_policy_criteria_met,
                    "claim_eligible": report.claim_eligible,
                    "invalid_run_count": report.invalid_run_count,
                }))
            }
            AgentTokenCommand::Publish(args) => {
                let manifest = load_agent_token_campaign_for_evidence(&args.manifest)?;
                let schedule = load_agent_token_schedule(
                    &args.campaign_dir.join("randomization-schedule.json"),
                )?;
                let evidence_errors =
                    validate_agent_token_campaign_evidence(&manifest, &args.campaign_dir)?;
                if !evidence_errors.is_empty() {
                    return Err(format!(
                        "Agent-token evidence failed validation: {}",
                        evidence_errors.join("; ")
                    ));
                }
                let statistical_view = load_agent_token_campaign_statistical_view(
                    &manifest,
                    &schedule,
                    &args.campaign_dir,
                )?;
                let report = &statistical_view.report;
                let campaign_manifest = args.campaign_dir.join("campaign-manifest.json");
                let protocol = args.campaign_dir.join("protocol.json");
                let fixture_manifest = args.campaign_dir.join("fixture-manifest.json");
                let randomization_schedule = args.campaign_dir.join("randomization-schedule.json");
                let raw_run_index = args.campaign_dir.join("raw-run-index.json");
                let mut source_files = vec![
                    ("campaign-manifest.json", campaign_manifest.as_path()),
                    ("fixture-manifest.json", fixture_manifest.as_path()),
                    ("protocol.json", protocol.as_path()),
                    (
                        "randomization-schedule.json",
                        randomization_schedule.as_path(),
                    ),
                    ("raw-run-index.json", raw_run_index.as_path()),
                ];
                let replacement_selection = args.campaign_dir.join("statistical-replacement.json");
                let replacement_runner = args
                    .campaign_dir
                    .join("statistical-replacements/replacement-0001/replacement-runner");
                let replacement_summary = statistical_view
                    .selection
                    .as_ref()
                    .map(|selection| args.campaign_dir.join(&selection.replacement_run_summary));
                if replacement_selection.is_file() {
                    source_files.push((
                        "statistical-replacement.json",
                        replacement_selection.as_path(),
                    ));
                }
                if let Some(path) = replacement_summary.as_deref() {
                    source_files.push(("replacement-run-summary.json", path));
                }
                if replacement_runner.is_file() {
                    source_files.push(("replacement-runner", replacement_runner.as_path()));
                }
                let receipt = write_agent_token_publication_bundle(AgentTokenPublicationInput {
                    output_dir: &args.output_dir,
                    release_version: &args.release_version,
                    measured_product_snapshot: &args.measured_product_snapshot,
                    measured_ait_executable_sha256: &args.measured_ait_sha256,
                    campaign_runner_sha256: &args.campaign_runner_sha256,
                    manifest: &manifest,
                    report,
                    runs: &statistical_view.effective_runs,
                    excluded_runs: &statistical_view.excluded_runs,
                    run_summary_paths: &statistical_view.effective_run_summary_paths,
                    excluded_run_summary_paths: &statistical_view.excluded_run_summary_paths,
                    source_files: &source_files,
                })?;
                emit_json(&receipt)
            }
            AgentTokenCommand::Compare(args) => {
                let baseline = load_agent_token_report(&args.baseline_report)?;
                let candidate = load_agent_token_report(&args.candidate_report)?;
                let comparison = compare_agent_token_reports(&baseline, &candidate);
                write_json_new(&args.output_json, &comparison)?;
                let comparable = comparison.comparable;
                emit_json(&comparison)?;
                if comparable {
                    Ok(())
                } else {
                    Err("Agent-token campaign reports are not comparable".to_string())
                }
            }
        },
    }
}

fn history_node_count(vcs: ProbeVcs, program: &PathBuf, root: &PathBuf) -> Result<u64, String> {
    let mut command = Command::new(program);
    command.current_dir(root);
    match vcs {
        ProbeVcs::Ait => {
            command.args(["snapshot", "list", "--json"]);
        }
        ProbeVcs::Git => {
            command.args(["rev-list", "--count", "HEAD"]);
        }
    }
    let output = command.output().map_err(|error| {
        format!(
            "Failed to launch history probe {} in {}: {error}",
            program.display(),
            root.display()
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "History probe {} failed with {:?}: {}",
            program.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    match vcs {
        ProbeVcs::Ait => {
            let snapshots = serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout)
                .map_err(|error| format!("AIT history probe did not emit a JSON array: {error}"))?;
            u64::try_from(snapshots.len())
                .map_err(|error| format!("AIT history count exceeds u64: {error}"))
        }
        ProbeVcs::Git => String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .map_err(|error| format!("Git history probe did not emit one integer: {error}")),
    }
}

fn load_and_validate(path: &Path) -> Result<(ait_benchmark::BenchmarkManifest, String), String> {
    let (manifest, digest) = load_manifest(path)?;
    let validation = validate_manifest(&manifest, &digest);
    if !validation.valid {
        return Err(format!(
            "Benchmark manifest failed validation: {}",
            validation.errors.join("; ")
        ));
    }
    Ok((manifest, digest))
}

fn emit_json(value: &impl Serialize) -> Result<(), String> {
    let output = serde_json::to_string_pretty(value)
        .map_err(|error| format!("Failed to encode command output: {error}"))?;
    writeln!(io::stdout(), "{output}")
        .map_err(|error| format!("Failed to write command output: {error}"))
}
