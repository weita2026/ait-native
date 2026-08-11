use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use ait_benchmark::{
    build_report, compare_reports, create_synthetic_fixture, digest_workspace,
    load_benchmark_report, load_budget_manifest, load_manifest, run_benchmark, validate_manifest,
    write_comparison_report, write_report, RunOptions, SyntheticFixtureRecipe, PROTOCOL_V1_JSON,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(
    name = "ait-benchmark",
    version,
    about = "Run the Python-free, versioned AIT/Git VCS benchmark protocol"
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
}

#[derive(Debug, Args)]
struct ManifestArgs {
    #[arg(long)]
    manifest: PathBuf,
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
        BenchmarkCommand::Run(args) => {
            let (manifest, digest) = load_and_validate(&args.manifest)?;
            let summary = run_benchmark(
                &manifest,
                &digest,
                &args.raw_jsonl,
                RunOptions { smoke: args.smoke },
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
