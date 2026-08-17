use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread;
use std::time::Duration;

use ait_agent_core::{AgentManagementRuntime, AgentSupervisorAction, TransportKind};
use ait_core::file_io::FilesystemFileIoStore;
use ait_core::json_support::{
    expand_home_path_with_file_io_store, json, JsonCodec, JsonEncodeOptions, JsonValue,
};
use clap::Parser;

mod commands;

pub use commands::AgentCommand;
use commands::{
    DiscordAddArgs, DiscordCommand, LineAddArgs, LineCommand, LogsArgs, NamedJsonArgs,
    SlackAddArgs, SlackCommand, StatusArgs, TelegramArgs, TelegramCommand, TelegramConsoleMode,
    TelegramSupervisorCommand, TelegramSupervisorRunArgs, WorkerPathArgs,
};

#[derive(Parser, Debug)]
#[command(name = "ait-agent", version)]
#[command(about = "Run optional ait external runtime workers.")]
struct AgentCli {
    #[command(subcommand)]
    command: AgentCommand,
}

pub fn entry() -> ExitCode {
    match run_command(AgentCli::parse().command) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::from(2)
        }
    }
}

pub fn run_command(command: AgentCommand) -> Result<ExitCode, String> {
    let runtime = management_runtime()?;
    match command {
        AgentCommand::Telegram(args) => run_telegram(&runtime, args),
        AgentCommand::Line { command } => {
            run_line(&runtime, command)?;
            Ok(ExitCode::SUCCESS)
        }
        AgentCommand::Discord { command } => {
            run_discord(&runtime, command)?;
            Ok(ExitCode::SUCCESS)
        }
        AgentCommand::Slack { command } => {
            run_slack(&runtime, command)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn run_telegram(runtime: &AgentManagementRuntime, args: TelegramArgs) -> Result<ExitCode, String> {
    let Some(command) = args.command else {
        return run_telegram_foreground(runtime, args.mode, &args.worker);
    };
    match command {
        TelegramCommand::Add(args) => {
            let payload = runtime.add_worker(with_paths(
                json!({
                    "kind": "telegram",
                    "name": args.name.trim(),
                    "token": args.token,
                    "username": args.username,
                }),
                args.paths,
            )?)?;
            emit_payload(&payload, args.json)?;
        }
        TelegramCommand::List(args) => {
            emit_workers(runtime.list_workers(TransportKind::Telegram)?, args.json)?;
        }
        TelegramCommand::Status(args) => run_status(runtime, TransportKind::Telegram, args)?,
        TelegramCommand::Start(args) => run_start(runtime, TransportKind::Telegram, args)?,
        TelegramCommand::Stop(args) => run_stop(runtime, TransportKind::Telegram, args)?,
        TelegramCommand::Restart(args) => run_restart(runtime, TransportKind::Telegram, args)?,
        TelegramCommand::Logs(args) => run_logs(runtime, TransportKind::Telegram, args)?,
        TelegramCommand::Remove(args) => run_remove(runtime, TransportKind::Telegram, args)?,
        TelegramCommand::Supervisor { command } => {
            run_telegram_supervisor(runtime, command)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_telegram_foreground(
    runtime: &AgentManagementRuntime,
    mode: TelegramConsoleMode,
    worker: &str,
) -> Result<ExitCode, String> {
    let mut argv = runtime.foreground_worker_command(TransportKind::Telegram, worker)?;
    if mode == TelegramConsoleMode::Webhook {
        argv.push("--console-mode".to_string());
        argv.push("webhook".to_string());
    }
    execute_foreground_worker(argv)
}

fn execute_foreground_worker(argv: Vec<String>) -> Result<ExitCode, String> {
    let (binary, args) = argv.split_first().ok_or_else(|| {
        "Rust launch contract returned an empty command; refusing Python fallback.".to_string()
    })?;
    let mut command = Command::new(binary);
    command.args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let error = command.exec();
        Err(format!(
            "Failed to execute native ait-agent worker `{binary}`: {error}"
        ))
    }
    #[cfg(not(unix))]
    {
        let status = command.status().map_err(|error| {
            format!("Failed to run native ait-agent worker `{binary}`: {error}")
        })?;
        match status.code() {
            Some(0) => Ok(ExitCode::SUCCESS),
            Some(code) => std::process::exit(code),
            None => Ok(ExitCode::from(1)),
        }
    }
}

fn run_line(runtime: &AgentManagementRuntime, command: LineCommand) -> Result<(), String> {
    match command {
        LineCommand::Add(args) => add_line(runtime, args),
        LineCommand::List(args) => {
            emit_workers(runtime.list_workers(TransportKind::Line)?, args.json)
        }
        LineCommand::Status(args) => run_status(runtime, TransportKind::Line, args),
        LineCommand::Start(args) => run_start(runtime, TransportKind::Line, args),
        LineCommand::Stop(args) => run_stop(runtime, TransportKind::Line, args),
        LineCommand::Restart(args) => run_restart(runtime, TransportKind::Line, args),
        LineCommand::Logs(args) => run_logs(runtime, TransportKind::Line, args),
        LineCommand::Remove(args) => run_remove(runtime, TransportKind::Line, args),
    }
}

fn run_discord(runtime: &AgentManagementRuntime, command: DiscordCommand) -> Result<(), String> {
    match command {
        DiscordCommand::Add(args) => add_discord(runtime, args),
        DiscordCommand::List(args) => {
            emit_workers(runtime.list_workers(TransportKind::Discord)?, args.json)
        }
        DiscordCommand::Status(args) => run_status(runtime, TransportKind::Discord, args),
        DiscordCommand::Start(args) => run_start(runtime, TransportKind::Discord, args),
        DiscordCommand::Stop(args) => run_stop(runtime, TransportKind::Discord, args),
        DiscordCommand::Restart(args) => run_restart(runtime, TransportKind::Discord, args),
        DiscordCommand::Logs(args) => run_logs(runtime, TransportKind::Discord, args),
        DiscordCommand::Remove(args) => run_remove(runtime, TransportKind::Discord, args),
    }
}

fn run_slack(runtime: &AgentManagementRuntime, command: SlackCommand) -> Result<(), String> {
    match command {
        SlackCommand::Add(args) => add_slack(runtime, args),
        SlackCommand::List(args) => {
            emit_workers(runtime.list_workers(TransportKind::Slack)?, args.json)
        }
        SlackCommand::Status(args) => run_status(runtime, TransportKind::Slack, args),
        SlackCommand::Start(args) => run_start(runtime, TransportKind::Slack, args),
        SlackCommand::Stop(args) => run_stop(runtime, TransportKind::Slack, args),
        SlackCommand::Restart(args) => run_restart(runtime, TransportKind::Slack, args),
        SlackCommand::Logs(args) => run_logs(runtime, TransportKind::Slack, args),
        SlackCommand::Remove(args) => run_remove(runtime, TransportKind::Slack, args),
    }
}

fn add_line(runtime: &AgentManagementRuntime, args: LineAddArgs) -> Result<(), String> {
    let payload = runtime.add_worker(with_paths(
        json!({
            "kind": "line",
            "name": args.name.trim(),
            "token": args.token,
            "secret": args.secret,
        }),
        args.paths,
    )?)?;
    emit_payload(&payload, args.json)
}

fn add_discord(runtime: &AgentManagementRuntime, args: DiscordAddArgs) -> Result<(), String> {
    let payload = runtime.add_worker(with_paths(
        json!({
            "kind": "discord",
            "name": args.name.trim(),
            "application_id": args.application_id,
            "bot_token": args.bot_token,
        }),
        args.paths,
    )?)?;
    emit_payload(&payload, args.json)
}

fn add_slack(runtime: &AgentManagementRuntime, args: SlackAddArgs) -> Result<(), String> {
    let payload = runtime.add_worker(with_paths(
        json!({
            "kind": "slack",
            "name": args.name.trim(),
            "app_token": args.app_token,
        }),
        args.paths,
    )?)?;
    emit_payload(&payload, args.json)
}

fn run_status(
    runtime: &AgentManagementRuntime,
    transport: TransportKind,
    args: StatusArgs,
) -> Result<(), String> {
    let payload = runtime.status_workers(transport, args.name.as_deref())?;
    if payload.is_array() {
        emit_workers(payload.as_array().cloned().unwrap_or_default(), args.json)
    } else {
        emit_payload(&payload, args.json)
    }
}

fn run_start(
    runtime: &AgentManagementRuntime,
    transport: TransportKind,
    args: NamedJsonArgs,
) -> Result<(), String> {
    emit_payload(&runtime.start_worker(transport, &args.name)?, args.json)
}

fn run_stop(
    runtime: &AgentManagementRuntime,
    transport: TransportKind,
    args: NamedJsonArgs,
) -> Result<(), String> {
    emit_payload(&runtime.stop_worker(transport, &args.name)?, args.json)
}

fn run_restart(
    runtime: &AgentManagementRuntime,
    transport: TransportKind,
    args: NamedJsonArgs,
) -> Result<(), String> {
    emit_payload(&runtime.restart_worker(transport, &args.name)?, args.json)
}

fn run_logs(
    runtime: &AgentManagementRuntime,
    transport: TransportKind,
    args: LogsArgs,
) -> Result<(), String> {
    let payload = runtime.worker_logs(transport, &args.name, args.lines)?;
    if args.json {
        return print_json(&payload);
    }
    let lines = payload
        .get("lines")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        println!("No log lines available for {}.", args.name);
    } else {
        for line in lines {
            println!("{line}");
        }
    }
    Ok(())
}

fn run_remove(
    runtime: &AgentManagementRuntime,
    transport: TransportKind,
    args: NamedJsonArgs,
) -> Result<(), String> {
    emit_payload(&runtime.remove_worker(transport, &args.name)?, args.json)
}

fn run_telegram_supervisor(
    runtime: &AgentManagementRuntime,
    command: TelegramSupervisorCommand,
) -> Result<(), String> {
    match command {
        TelegramSupervisorCommand::Status(args) => emit_payload(
            &runtime.telegram_supervisor(AgentSupervisorAction::Status, None, None)?,
            args.json,
        ),
        TelegramSupervisorCommand::Start(args) => emit_payload(
            &runtime.telegram_supervisor(AgentSupervisorAction::Start, None, None)?,
            args.json,
        ),
        TelegramSupervisorCommand::Stop(args) => emit_payload(
            &runtime.telegram_supervisor(AgentSupervisorAction::Stop, None, None)?,
            args.json,
        ),
        TelegramSupervisorCommand::Restart(args) => emit_payload(
            &runtime.telegram_supervisor(AgentSupervisorAction::Restart, None, None)?,
            args.json,
        ),
        TelegramSupervisorCommand::Run(args) => run_telegram_supervisor_loop(runtime, args),
    }
}

fn run_telegram_supervisor_loop(
    runtime: &AgentManagementRuntime,
    args: TelegramSupervisorRunArgs,
) -> Result<(), String> {
    let mut cycle = 0usize;
    loop {
        cycle += 1;
        let payload = runtime.telegram_supervisor(
            AgentSupervisorAction::Run,
            Some(args.interval_seconds),
            Some(cycle),
        )?;
        emit_payload(&payload, args.json)?;
        if args.once {
            return Ok(());
        }
        thread::sleep(Duration::from_secs_f64(args.interval_seconds));
    }
}

fn with_paths(mut worker: JsonValue, paths: WorkerPathArgs) -> Result<JsonValue, String> {
    let object = worker
        .as_object_mut()
        .ok_or_else(|| "worker payload must be an object".to_string())?;
    object.insert(
        "sync_state_path".to_string(),
        optional_string(paths.sync_state_path),
    );
    object.insert("pid_file".to_string(), optional_string(paths.pid_file));
    object.insert("log_file".to_string(), optional_string(paths.log_file));
    object.insert("env_path".to_string(), optional_string(paths.env_path));
    Ok(worker)
}

fn optional_string(value: Option<String>) -> JsonValue {
    value.map(JsonValue::String).unwrap_or(JsonValue::Null)
}

fn emit_payload(payload: &JsonValue, _json: bool) -> Result<(), String> {
    print_json(payload)
}

fn emit_workers(workers: Vec<JsonValue>, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(&JsonValue::Array(workers));
    }
    for worker in workers {
        let kind = worker
            .get("kind")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        let name = worker
            .get("name")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        let username = worker
            .get("username")
            .and_then(JsonValue::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("-");
        let token_preview = worker
            .get("token_preview")
            .and_then(JsonValue::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("-");
        println!("{kind}/{name}\t{username}\t{token_preview}");
    }
    Ok(())
}

fn print_json(payload: &JsonValue) -> Result<(), String> {
    let text = JsonCodec::encode_value(payload, JsonEncodeOptions::pretty())
        .map_err(|error| format!("Failed to encode ait-agent output: {error}"))?;
    println!("{text}");
    Ok(())
}

fn management_runtime() -> Result<AgentManagementRuntime, String> {
    let repo_root = resolve_repo_root()?;
    let manifest_path = resolve_manifest_path(&repo_root)?;
    let worker_binary = resolve_worker_binary();
    let parent_env = env::vars().collect::<BTreeMap<_, _>>();
    Ok(AgentManagementRuntime::filesystem(
        repo_root,
        manifest_path,
        worker_binary,
        parent_env,
    ))
}

fn resolve_repo_root() -> Result<PathBuf, String> {
    let cwd =
        env::current_dir().map_err(|error| format!("Failed to read current directory: {error}"))?;
    let raw = env::var_os("AIT_REPO_ROOT")
        .or_else(|| env::var_os("AIT_NATIVE_WORKSPACE_ROOT"))
        .or_else(|| env::var_os("AIT_WORKSPACE_ROOT"))
        .map(PathBuf::from)
        .unwrap_or(cwd.clone());
    Ok(resolve_absolute_path(&cwd, raw))
}

fn resolve_manifest_path(repo_root: &Path) -> Result<PathBuf, String> {
    let path = match env::var("AIT_AGENT_CONFIG_PATH") {
        Ok(value) if !value.trim().is_empty() => {
            expand_home_path_with_file_io_store(&FilesystemFileIoStore, value.trim())
        }
        _ => repo_root.join(".ait/agent-workers.json"),
    };
    let cwd =
        env::current_dir().map_err(|error| format!("Failed to read current directory: {error}"))?;
    Ok(resolve_absolute_path(&cwd, path))
}

fn resolve_absolute_path(cwd: &Path, path: PathBuf) -> PathBuf {
    let absolute = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    absolute.canonicalize().unwrap_or(absolute)
}

fn resolve_worker_binary() -> String {
    if let Ok(value) = env::var("AIT_AGENT_RUST_WORKER_BINARY") {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    if let Ok(current_executable) = env::current_exe() {
        if let Some(parent) = current_executable.parent() {
            let candidate = parent.join(format!("ait-agent-worker{}", env::consts::EXE_SUFFIX));
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    "ait-agent-worker".to_string()
}
