use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use ait_runner::{
    ExecutorConfig, NativeExecutor, NativeJobRequest, RepositoryIndex, RunJobOptions, RunnerError,
    ServeOptions, ServerClient, WorkerJobIndex, WorkerJobKey,
};
use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::{Value as JsonValue, json};

#[derive(Debug, Parser)]
#[command(
    name = "ait-runner",
    version,
    about = "Native execution plane for explicit ait-server CI jobs"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Verify the live ait-server health contract without claiming a job.
    Doctor {
        #[arg(long, env = "AIT_SERVER_URL")]
        server: String,
    },
    /// Execute one typed native request from stdin or a named file.
    Execute {
        #[arg(long, default_value = "-")]
        request: String,
        #[arg(long, default_value = ".")]
        source_root: PathBuf,
        #[arg(long, env = "AIT_RUNNER_ATTEMPT_ROOT")]
        attempt_root: Option<PathBuf>,
    },
    /// Claim, execute, and finish one explicit Binary Worker Job pair.
    RunJob {
        #[arg(long, env = "AIT_SERVER_URL")]
        server: String,
        #[arg(long)]
        repository_index: u32,
        #[arg(long)]
        worker_job_index: u32,
        #[arg(long, default_value = ".")]
        source_root: PathBuf,
        #[arg(long, env = "AIT_RUNNER_ATTEMPT_ROOT")]
        attempt_root: Option<PathBuf>,
    },
    /// Poll for compatible native jobs, maintain their lease, and deliver results.
    Serve {
        #[arg(long, env = "AIT_SERVER_URL")]
        server: String,
        #[arg(long, env = "AIT_RUNNER_WORKER_ID")]
        worker_id: Option<String>,
        #[arg(long = "repository-index")]
        repository_indexes: Vec<u32>,
        #[arg(long)]
        once: bool,
        #[arg(long, default_value_t = 1_000)]
        poll_interval_ms: u64,
        #[arg(long, default_value_t = 30_000)]
        heartbeat_interval_ms: u64,
        #[arg(long, default_value = ".")]
        source_root: PathBuf,
        #[arg(long, env = "AIT_RUNNER_ATTEMPT_ROOT")]
        attempt_root: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(payload) => match write_json(io::stdout().lock(), &payload) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                let _ = write_json(io::stderr().lock(), &error_payload(&error));
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            let _ = write_json(io::stderr().lock(), &error_payload(&error));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<JsonValue, RunnerError> {
    match cli.command {
        Command::Doctor { server } => server_client(&server)?.doctor(),
        Command::Execute {
            request,
            source_root,
            attempt_root,
        } => {
            let bytes = read_request(&request)?;
            let request = NativeJobRequest::parse_bounded(&bytes)?;
            let executor = executor(source_root, attempt_root);
            serde_json::to_value(executor.execute(&request)?).map_err(|error| {
                RunnerError::Process(format!("could not encode native result: {error}"))
            })
        }
        Command::RunJob {
            server,
            repository_index,
            worker_job_index,
            source_root,
            attempt_root,
        } => {
            let executor = executor(source_root, attempt_root);
            server_client(&server)?.run_job(
                &executor,
                &RunJobOptions {
                    key: WorkerJobKey::new(
                        RepositoryIndex::new(repository_index),
                        WorkerJobIndex::new(worker_job_index),
                    ),
                },
            )
        }
        Command::Serve {
            server,
            worker_id,
            repository_indexes,
            once,
            poll_interval_ms,
            heartbeat_interval_ms,
            source_root,
            attempt_root,
        } => {
            let executor = executor(source_root, attempt_root);
            server_client(&server)?.serve(
                &executor,
                &ServeOptions {
                    worker_id: worker_id.unwrap_or_else(default_worker_id),
                    repository_indexes: normalize_repository_indexes(repository_indexes),
                    once,
                    poll_interval: Duration::from_millis(poll_interval_ms),
                    heartbeat_interval: Duration::from_millis(heartbeat_interval_ms),
                },
            )
        }
    }
}

fn normalize_repository_indexes(indexes: Vec<u32>) -> Vec<RepositoryIndex> {
    let mut indexes = indexes
        .into_iter()
        .map(RepositoryIndex::new)
        .collect::<Vec<_>>();
    indexes.sort_unstable();
    indexes.dedup();
    indexes
}

fn server_client(server: &str) -> Result<ServerClient, RunnerError> {
    let token = std::env::var("AIT_SERVER_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    ServerClient::new(server, token)
}

fn executor(source_root: PathBuf, attempt_root: Option<PathBuf>) -> NativeExecutor {
    NativeExecutor::new(ExecutorConfig {
        source_root,
        attempt_root: attempt_root.unwrap_or_else(default_attempt_root),
    })
}

fn default_attempt_root() -> PathBuf {
    std::env::temp_dir().join("ait-runner").join("attempts")
}

fn default_worker_id() -> String {
    let host = std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "host".to_string());
    format!("ait-runner-{host}-{}", std::process::id())
}

fn read_request(request: &str) -> Result<Vec<u8>, RunnerError> {
    if request == "-" {
        return read_bounded(io::stdin().lock(), Path::new("<stdin>"));
    }
    let path = PathBuf::from(request);
    let file = File::open(&path).map_err(|error| RunnerError::fs("open request", &path, error))?;
    read_bounded(file, &path)
}

fn read_bounded<R: Read>(reader: R, path: &Path) -> Result<Vec<u8>, RunnerError> {
    let mut bytes = Vec::new();
    reader
        .take((ait_runner::protocol::MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| RunnerError::fs("read request", path, error))?;
    if bytes.len() > ait_runner::protocol::MAX_REQUEST_BYTES {
        return Err(RunnerError::InvalidRequest(format!(
            "request exceeds {} bytes",
            ait_runner::protocol::MAX_REQUEST_BYTES
        )));
    }
    Ok(bytes)
}

fn error_payload(error: &RunnerError) -> JsonValue {
    json!({
        "contract": "ait.runner.error.v1",
        "status": "error",
        "error": error.to_string(),
    })
}

fn write_json<W: io::Write, T: Serialize>(mut writer: W, payload: &T) -> Result<(), RunnerError> {
    serde_json::to_writer(&mut writer, payload)
        .map_err(|error| RunnerError::Process(format!("could not write JSON output: {error}")))?;
    writeln!(writer)
        .map_err(|error| RunnerError::Process(format!("could not finish JSON output: {error}")))
}
