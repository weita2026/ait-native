use clap::{Args, Subcommand, ValueEnum};

#[derive(Subcommand, Debug)]
pub enum AgentCommand {
    #[command(about = "Run and configure Telegram agent workers.")]
    Telegram(TelegramArgs),
    #[command(about = "Run and configure LINE agent workers.")]
    Line {
        #[command(subcommand)]
        command: LineCommand,
    },
    #[command(about = "Run and configure Discord agent workers.")]
    Discord {
        #[command(subcommand)]
        command: DiscordCommand,
    },
    #[command(about = "Run and configure Slack agent workers.")]
    Slack {
        #[command(subcommand)]
        command: SlackCommand,
    },
}

#[derive(Args, Debug)]
pub struct TelegramArgs {
    #[arg(long, value_enum, default_value_t = TelegramConsoleMode::Polling)]
    pub mode: TelegramConsoleMode,
    #[arg(long, default_value = "main")]
    pub worker: String,
    #[command(subcommand)]
    pub command: Option<TelegramCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TelegramConsoleMode {
    Polling,
    Webhook,
}

#[derive(Subcommand, Debug)]
pub enum TelegramCommand {
    #[command(about = "Add or update a named Telegram worker config.")]
    Add(TelegramAddArgs),
    #[command(about = "List named Telegram worker configs.")]
    List(JsonArgs),
    #[command(about = "Show named Telegram worker runtime status.")]
    Status(StatusArgs),
    #[command(about = "Start a named Telegram worker in daemon mode.")]
    Start(NamedJsonArgs),
    #[command(about = "Stop a named Telegram worker.")]
    Stop(NamedJsonArgs),
    #[command(about = "Stop and restart a named Telegram worker in daemon mode.")]
    Restart(NamedJsonArgs),
    #[command(about = "Read the last N lines from a named Telegram worker's log.")]
    Logs(LogsArgs),
    #[command(about = "Remove a named Telegram worker config.")]
    Remove(NamedJsonArgs),
    #[command(about = "Supervise configured Telegram workers.")]
    Supervisor {
        #[command(subcommand)]
        command: TelegramSupervisorCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum LineCommand {
    #[command(about = "Add or update a named LINE worker config.")]
    Add(LineAddArgs),
    List(JsonArgs),
    Status(StatusArgs),
    Start(NamedJsonArgs),
    Stop(NamedJsonArgs),
    Restart(NamedJsonArgs),
    Logs(LogsArgs),
    Remove(NamedJsonArgs),
}

#[derive(Subcommand, Debug)]
pub enum DiscordCommand {
    #[command(about = "Add or update a named Discord worker config.")]
    Add(DiscordAddArgs),
    List(JsonArgs),
    Status(StatusArgs),
    Start(NamedJsonArgs),
    Stop(NamedJsonArgs),
    Restart(NamedJsonArgs),
    Logs(LogsArgs),
    Remove(NamedJsonArgs),
}

#[derive(Subcommand, Debug)]
pub enum SlackCommand {
    #[command(about = "Add or update a named Slack worker config.")]
    Add(SlackAddArgs),
    List(JsonArgs),
    Status(StatusArgs),
    Start(NamedJsonArgs),
    Stop(NamedJsonArgs),
    Restart(NamedJsonArgs),
    Logs(LogsArgs),
    Remove(NamedJsonArgs),
}

#[derive(Subcommand, Debug)]
pub enum TelegramSupervisorCommand {
    #[command(about = "Show runtime status for all Telegram workers.")]
    Status(JsonArgs),
    #[command(about = "Start all configured Telegram workers.")]
    Start(JsonArgs),
    #[command(about = "Continuously start stopped Telegram workers at an interval.")]
    Run(TelegramSupervisorRunArgs),
    #[command(about = "Stop all running Telegram workers.")]
    Stop(JsonArgs),
    #[command(about = "Restart all Telegram workers.")]
    Restart(JsonArgs),
}

#[derive(Args, Debug)]
pub struct JsonArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct NamedJsonArgs {
    pub name: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    pub name: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct LogsArgs {
    pub name: String,
    #[arg(long, default_value_t = 100, value_parser = parse_log_lines)]
    pub lines: usize,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct WorkerPathArgs {
    #[arg(long = "sync-state-path")]
    pub sync_state_path: Option<String>,
    #[arg(long = "pid-file")]
    pub pid_file: Option<String>,
    #[arg(long = "log-file")]
    pub log_file: Option<String>,
    #[arg(long = "env-path")]
    pub env_path: Option<String>,
}

#[derive(Args, Debug)]
pub struct TelegramAddArgs {
    pub name: String,
    #[arg(long)]
    pub token: String,
    #[arg(long)]
    pub username: Option<String>,
    #[command(flatten)]
    pub paths: WorkerPathArgs,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct LineAddArgs {
    pub name: String,
    #[arg(long)]
    pub token: String,
    #[arg(long)]
    pub secret: String,
    #[command(flatten)]
    pub paths: WorkerPathArgs,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct DiscordAddArgs {
    pub name: String,
    #[arg(long = "application-id")]
    pub application_id: String,
    #[arg(long = "bot-token")]
    pub bot_token: String,
    #[command(flatten)]
    pub paths: WorkerPathArgs,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SlackAddArgs {
    pub name: String,
    #[arg(long = "app-token")]
    pub app_token: String,
    #[command(flatten)]
    pub paths: WorkerPathArgs,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct TelegramSupervisorRunArgs {
    #[arg(long = "interval-seconds", default_value_t = 30.0, value_parser = parse_interval_seconds)]
    pub interval_seconds: f64,
    #[arg(long)]
    pub once: bool,
    #[arg(long)]
    pub json: bool,
}

fn parse_interval_seconds(value: &str) -> Result<f64, String> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| "interval-seconds must be a number".to_string())?;
    if !parsed.is_finite() || parsed < 1.0 {
        return Err("interval-seconds must be at least 1".to_string());
    }
    Ok(parsed)
}

fn parse_log_lines(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| "lines must be an integer".to_string())?;
    if !(1..=10_000).contains(&parsed) {
        return Err("lines must be between 1 and 10000".to_string());
    }
    Ok(parsed)
}
