use std::collections::BTreeMap;
use std::env;
use std::io::{self, Read, Write};
use std::process::ExitCode;

use ait_agent_worker::{
    compiled_worker_capabilities, configure_native_reply_provider,
    execute_discord_interaction_once, execute_native_reply_provider, execute_slack_command_once,
    execute_telegram_webhook_once, execute_worker_request, prepare_worker_run,
    process_worker_path_inputs, render_capabilities_json, render_capabilities_text,
    DiscordInteractionOnceRequest, SlackCommandOnceRequest, TransportRunnerRegistry,
    WorkerDiagnostic, WorkerRunRequest, EXIT_INVALID_REQUEST, EXIT_RUNTIME_UNAVAILABLE,
};
use ait_core::json_support::{JsonCodec, JsonEncodeOptions, JsonValue};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "ait-agent-worker",
    version,
    about = "Run Python-free AIT transport workers"
)]
struct Cli {
    #[command(subcommand)]
    command: WorkerCommand,
}

#[derive(Debug, Subcommand)]
enum WorkerCommand {
    /// Report transport runners compiled into this executable.
    Capabilities(CapabilitiesArgs),
    /// Run one configured transport worker.
    Run(RunArgs),
    /// Execute one signed Slack slash-command request from stdin.
    SlackCommand(SlackCommandArgs),
    /// Execute one signed Discord interaction request from stdin.
    DiscordInteraction(DiscordInteractionArgs),
    /// Generate one native Codex reply from a versioned provider request on stdin.
    ReplyProvider,
}

#[derive(Debug, Args)]
struct CapabilitiesArgs {
    /// Emit the stable machine-readable capability contract.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct RunArgs {
    /// Transport kind: telegram, discord, slack, or line.
    #[arg(long)]
    transport: String,
    /// Named worker from .ait/agent-workers.json.
    #[arg(long)]
    worker: String,
    /// Rust event-loop backend selected by launch planning.
    #[arg(long)]
    event_loop_backend: String,
    /// Zero-based Rust reactor shard assignment.
    #[arg(long)]
    shard: String,
    /// Run the long-lived service or one stdin Telegram webhook transaction.
    #[arg(long, value_enum, default_value_t = RunConsoleMode::Service)]
    console_mode: RunConsoleMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum RunConsoleMode {
    Service,
    Webhook,
}

#[derive(Debug, Args)]
struct SlackCommandArgs {
    /// Named Slack worker from .ait/agent-workers.json.
    #[arg(long, default_value = "main")]
    worker: String,
}

#[derive(Debug, Args)]
struct DiscordInteractionArgs {
    /// Named Discord worker from .ait/agent-workers.json.
    #[arg(long, default_value = "main")]
    worker: String,
}

fn main() -> ExitCode {
    match entry(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(diagnostic) => {
            let _ = io::stderr().write_all(diagnostic.render_json().as_bytes());
            ExitCode::from(diagnostic.exit_code)
        }
    }
}

fn entry(cli: Cli) -> Result<(), WorkerDiagnostic> {
    match cli.command {
        WorkerCommand::Capabilities(args) => {
            let capabilities = compiled_worker_capabilities();
            let output = if args.json {
                render_capabilities_json(&capabilities).map_err(|message| {
                    WorkerDiagnostic::new(
                        "capability_serialization_failed",
                        message,
                        EXIT_RUNTIME_UNAVAILABLE,
                    )
                })?
            } else {
                render_capabilities_text(&capabilities)
            };
            io::stdout().write_all(output.as_bytes()).map_err(|error| {
                WorkerDiagnostic::new(
                    "capability_output_failed",
                    format!("Failed to write ait-agent-worker capabilities: {error}"),
                    EXIT_RUNTIME_UNAVAILABLE,
                )
            })?;
            Ok(())
        }
        WorkerCommand::Run(args) => {
            configure_reply_provider(&args.transport)?;
            let path_inputs = process_worker_path_inputs()?;
            let request = WorkerRunRequest {
                transport: args.transport,
                worker: args.worker,
                event_loop_backend: args.event_loop_backend,
                shard: args.shard,
            };
            match args.console_mode {
                RunConsoleMode::Service => execute_worker_request(
                    &request,
                    &path_inputs,
                    &TransportRunnerRegistry::compiled(),
                ),
                RunConsoleMode::Webhook => {
                    let mut raw_payload = String::new();
                    io::stdin().read_to_string(&mut raw_payload).map_err(|_| {
                        WorkerDiagnostic::new(
                            "telegram_webhook_input_failed",
                            "Failed to read the Telegram webhook payload from stdin.",
                            EXIT_INVALID_REQUEST,
                        )
                    })?;
                    let context = prepare_worker_run(&request, &path_inputs)?;
                    let result = execute_telegram_webhook_once(&context, &raw_payload)?;
                    write_json_result(
                        &result,
                        "telegram_webhook_output_failed",
                        "Telegram webhook result",
                    )
                }
            }
        }
        WorkerCommand::SlackCommand(args) => {
            configure_reply_provider("slack")?;
            let mut raw_payload = String::new();
            io::stdin().read_to_string(&mut raw_payload).map_err(|_| {
                WorkerDiagnostic::new(
                    "slack_command_input_failed",
                    "Failed to read the Slack command payload from stdin.",
                    EXIT_INVALID_REQUEST,
                )
            })?;
            let process_env = env::vars().collect::<BTreeMap<_, _>>();
            let signature =
                first_env_text(&process_env, &["AIT_SLACK_SIGNATURE", "X_SLACK_SIGNATURE"]);
            let signature_timestamp = first_env_text(
                &process_env,
                &["AIT_SLACK_SIGNATURE_TIMESTAMP", "X_SLACK_REQUEST_TIMESTAMP"],
            );
            let result = execute_slack_command_once(&SlackCommandOnceRequest {
                path_inputs: process_worker_path_inputs()?,
                worker_name: args.worker,
                process_env,
                raw_payload,
                signature,
                signature_timestamp,
                now_unix_seconds: None,
            })?;
            let output = JsonCodec::encode_value(
                &result,
                JsonEncodeOptions::pretty().with_trailing_newline(),
            )
            .map_err(|_| {
                WorkerDiagnostic::new(
                    "slack_command_output_failed",
                    "Failed to serialize the Slack command result.",
                    EXIT_RUNTIME_UNAVAILABLE,
                )
            })?;
            io::stdout().write_all(output.as_bytes()).map_err(|_| {
                WorkerDiagnostic::new(
                    "slack_command_output_failed",
                    "Failed to write the Slack command result.",
                    EXIT_RUNTIME_UNAVAILABLE,
                )
            })?;
            Ok(())
        }
        WorkerCommand::DiscordInteraction(args) => {
            configure_reply_provider("discord")?;
            let mut raw_payload = String::new();
            io::stdin().read_to_string(&mut raw_payload).map_err(|_| {
                WorkerDiagnostic::new(
                    "discord_interaction_input_failed",
                    "Failed to read the Discord interaction payload from stdin.",
                    EXIT_INVALID_REQUEST,
                )
            })?;
            let process_env = env::vars().collect::<BTreeMap<_, _>>();
            let signature = first_env_text(
                &process_env,
                &["AIT_DISCORD_SIGNATURE", "AIT_DISCORD_INTERACTION_SIGNATURE"],
            );
            let signature_timestamp = first_env_text(
                &process_env,
                &[
                    "AIT_DISCORD_SIGNATURE_TIMESTAMP",
                    "AIT_DISCORD_INTERACTION_TIMESTAMP",
                ],
            );
            let result = execute_discord_interaction_once(&DiscordInteractionOnceRequest {
                path_inputs: process_worker_path_inputs()?,
                worker_name: args.worker,
                process_env,
                raw_payload,
                signature,
                signature_timestamp,
            })?;
            if result.get("ok").and_then(JsonValue::as_bool) != Some(true) {
                return Err(WorkerDiagnostic::new(
                    "discord_interaction_job_failed",
                    "Rust Discord interaction execution failed.",
                    EXIT_RUNTIME_UNAVAILABLE,
                ));
            }
            let response = result
                .get("response")
                .filter(|value| value.is_object())
                .ok_or_else(|| {
                    WorkerDiagnostic::new(
                        "discord_interaction_response_invalid",
                        "Rust Discord interaction execution omitted its response.",
                        EXIT_RUNTIME_UNAVAILABLE,
                    )
                })?;
            let output = JsonCodec::encode_value(
                response,
                JsonEncodeOptions::pretty().with_trailing_newline(),
            )
            .map_err(|_| {
                WorkerDiagnostic::new(
                    "discord_interaction_output_failed",
                    "Failed to serialize the Discord interaction response.",
                    EXIT_RUNTIME_UNAVAILABLE,
                )
            })?;
            io::stdout().write_all(output.as_bytes()).map_err(|_| {
                WorkerDiagnostic::new(
                    "discord_interaction_output_failed",
                    "Failed to write the Discord interaction response.",
                    EXIT_RUNTIME_UNAVAILABLE,
                )
            })?;
            Ok(())
        }
        WorkerCommand::ReplyProvider => {
            let mut raw_payload = String::new();
            io::stdin().read_to_string(&mut raw_payload).map_err(|_| {
                WorkerDiagnostic::new(
                    "reply_provider_input_failed",
                    "Failed to read the native reply provider request from stdin.",
                    EXIT_INVALID_REQUEST,
                )
            })?;
            let response = execute_native_reply_provider(
                &raw_payload,
                &env::vars().collect::<BTreeMap<_, _>>(),
            );
            let output = JsonCodec::encode_value(
                &response,
                JsonEncodeOptions::pretty().with_trailing_newline(),
            )
            .map_err(|_| {
                WorkerDiagnostic::new(
                    "reply_provider_output_failed",
                    "Failed to serialize the native reply provider response.",
                    EXIT_RUNTIME_UNAVAILABLE,
                )
            })?;
            io::stdout().write_all(output.as_bytes()).map_err(|_| {
                WorkerDiagnostic::new(
                    "reply_provider_output_failed",
                    "Failed to write the native reply provider response.",
                    EXIT_RUNTIME_UNAVAILABLE,
                )
            })?;
            Ok(())
        }
    }
}

fn configure_reply_provider(surface: &str) -> Result<(), WorkerDiagnostic> {
    configure_native_reply_provider(surface).map_err(|message| {
        WorkerDiagnostic::new(
            "reply_provider_configuration_failed",
            message,
            EXIT_RUNTIME_UNAVAILABLE,
        )
    })
}

fn write_json_result(
    result: &JsonValue,
    error_code: &'static str,
    result_name: &str,
) -> Result<(), WorkerDiagnostic> {
    let output =
        JsonCodec::encode_value(result, JsonEncodeOptions::pretty().with_trailing_newline())
            .map_err(|_| {
                WorkerDiagnostic::new(
                    error_code,
                    format!("Failed to serialize the {result_name}."),
                    EXIT_RUNTIME_UNAVAILABLE,
                )
            })?;
    io::stdout().write_all(output.as_bytes()).map_err(|_| {
        WorkerDiagnostic::new(
            error_code,
            format!("Failed to write the {result_name}."),
            EXIT_RUNTIME_UNAVAILABLE,
        )
    })
}

fn first_env_text(environment: &BTreeMap<String, String>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        environment
            .get(*name)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}
