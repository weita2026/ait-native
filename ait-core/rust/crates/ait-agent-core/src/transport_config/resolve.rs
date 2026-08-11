use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use ait_core::json_support::{json, JsonValue};

use crate::json_support::parse_value;
use crate::supervisor::{
    plan_worker_supervisor_lifecycle, AgentWorkerLifecycleOperation, AgentWorkerLifecyclePlanInput,
    AgentWorkerLifecycleSpec, AgentWorkerRuntimePaths,
};
use crate::transport::{
    agent_transport_config_clean_optional_text, agent_transport_config_normalize_base_url,
    TransportKind,
};

use super::types::{
    AgentRuntimeMode, AgentRuntimeTarget, AgentSecret, AgentSharedWorkerConfig,
    AgentWorkerRuntimeConfig, AgentWorkflowMode, DiscordWorkerConfig, LineWorkerConfig,
    SlackWorkerConfig, TelegramSttMode, TelegramWorkerConfig, TelegramWorkerMode,
};

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_MODEL: &str = "gpt-5.4-mini";
const DEFAULT_TELEGRAM_STT_MODEL: &str = "mlx-community/whisper-large-v3-mlx";
const DEFAULT_DISCORD_API_BASE_URL: &str = "https://discord.com/api/v10";
const DEFAULT_DISCORD_HTTP_USER_AGENT: &str = "curl/8.7.1";
const DEFAULT_LINE_API_BASE_URL: &str = "https://api.line.me";
const DEFAULT_SLACK_API_BASE_URL: &str = "https://slack.com/api";
const DEFAULT_SLACK_HTTP_USER_AGENT: &str = "ait-agent-worker/0.1";
const DEFAULT_SLACK_ACK_TEXT: &str = "ait is thinking...";
const DEFAULT_SLACK_RESPONSE_TYPE: &str = "in_channel";
const DEFAULT_TIMEOUT_DISABLE_TOKENS: &[&str] = &["inf", "infinite", "none"];
const TELEGRAM_TIMEOUT_DISABLE_TOKENS: &[&str] = &["inf", "infinite", "none", "null", "unlimited"];
const PLACEHOLDER_OPENAI_API_KEYS: &[&str] = &[
    "your-openai-api-key",
    "sk-your-openai-api-key",
    "your_openai_api_key",
    "replace-with-real-openai-api-key",
];

pub const AGENT_ENV_FILE_LOAD_CONTRACT: &str = "ait.agent.env_file_load.v1";
pub const AGENT_REPO_DEFAULT_MODEL_LOAD_CONTRACT: &str = "ait.agent.repo_default_model_load.v1";

#[derive(Clone)]
pub struct AgentWorkerConfigInput {
    pub repo_root: PathBuf,
    pub worker_key: String,
    pub worker: JsonValue,
    pub process_env: BTreeMap<String, String>,
}

pub fn parse_agent_env_file(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(format!(
                "failed to read ait-agent env file '{}': {error}",
                path.display()
            ))
        }
    };
    Ok(parse_agent_env_text(&raw))
}

pub fn agent_env_file_load_json(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request
        .as_object()
        .ok_or_else(|| "ait-agent env file load request must be an object".to_string())?;
    let path = required_request_text(request.get("path"), "env file load", "path")?;
    let path = Path::new(&path);
    let existed = path.exists();
    let values = parse_agent_env_file(path)?;
    Ok(json!({
        "contract": AGENT_ENV_FILE_LOAD_CONTRACT,
        "ok": true,
        "status": if existed { "loaded" } else { "not_found" },
        "path": path.to_string_lossy(),
        "values": values,
        "python_file_read_allowed": false,
    }))
}

pub fn agent_repo_default_model_load_json(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request.as_object().ok_or_else(|| {
        "ait-agent repository default-model load request must be an object".to_string()
    })?;
    let repo_root = required_request_text(
        request.get("repo_root"),
        "repository default-model load",
        "repo_root",
    )?;
    let config_path = Path::new(&repo_root).join(".ait").join("config.json");
    let (status, default_model) = match fs::read_to_string(&config_path) {
        Ok(raw) => match parse_value(&raw, "invalid ait-agent repository config") {
            Ok(JsonValue::Object(config)) => match clean_json_text(config.get("default_model")) {
                Some(model) => ("loaded", Some(model)),
                None if config.contains_key("default_model") => ("invalid_model", None),
                None => ("missing_model", None),
            },
            Ok(_) => ("invalid_payload", None),
            Err(_) => ("invalid_json", None),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ("not_found", None),
        Err(_) => ("unreadable", None),
    };
    Ok(json!({
        "contract": AGENT_REPO_DEFAULT_MODEL_LOAD_CONTRACT,
        "ok": true,
        "status": status,
        "repo_root": repo_root,
        "config_path": config_path.to_string_lossy(),
        "default_model": default_model,
        "python_file_read_allowed": false,
    }))
}

pub fn resolve_agent_worker_config(
    input: AgentWorkerConfigInput,
) -> Result<AgentWorkerRuntimeConfig, String> {
    let worker = input
        .worker
        .as_object()
        .ok_or_else(|| "ait-agent worker configuration must be a JSON object".to_string())?;
    let transport_text = clean_json_text(worker.get("kind"))
        .or_else(|| {
            input
                .worker_key
                .split_once('/')
                .map(|(kind, _)| kind.to_string())
        })
        .ok_or_else(|| "ait-agent worker configuration is missing kind".to_string())?;
    let transport = TransportKind::from_str(&transport_text)?;
    let worker_name = clean_json_text(worker.get("name"))
        .or_else(|| {
            input
                .worker_key
                .split_once('/')
                .map(|(_, name)| name.to_string())
        })
        .ok_or_else(|| "ait-agent worker configuration is missing name".to_string())?;
    let expected_key_prefix = format!("{}/", transport.as_str());
    if !input.worker_key.starts_with(&expected_key_prefix) {
        return Err("ait-agent worker key does not match its transport".to_string());
    }
    let lifecycle = plan_worker_supervisor_lifecycle(AgentWorkerLifecyclePlanInput {
        repo_root: input.repo_root.to_string_lossy().into_owned(),
        operation: AgentWorkerLifecycleOperation::Status,
        worker: AgentWorkerLifecycleSpec {
            transport,
            name: worker_name.clone(),
            sync_state_path: clean_json_text(worker.get("sync_state_path")),
            pid_file: clean_json_text(worker.get("pid_file")),
            log_file: clean_json_text(worker.get("log_file")),
            env_path: clean_json_text(worker.get("env_path")),
            termination_context_path: clean_json_text(worker.get("termination_context_path")),
        },
        runtime_root: clean_json_text(worker.get("runtime_root")),
        stop_timeout_seconds: None,
        kill_grace_seconds: None,
    })?;
    let mut paths = lifecycle.paths;
    let env_path_name = transport_env_path_name(transport);
    if let Some(value) = clean_map_text(input.process_env.get(env_path_name)) {
        paths.env_path = select_env_path(
            &input.repo_root,
            Path::new(&paths.env_path),
            &value,
            &input.process_env,
        )
        .to_string_lossy()
        .into_owned();
    }
    let env_file = parse_agent_env_file(Path::new(&paths.env_path))?;
    let mut process_env = input.process_env;
    overlay_manifest_credentials(transport, worker, &mut process_env);
    let sources = ConfigSources {
        process_env: &process_env,
        env_file: &env_file,
    };
    apply_runtime_path_overrides(
        transport,
        &input.repo_root,
        &sources,
        &process_env,
        &mut paths,
    );
    let repo_settings = resolve_repo_settings(&input.repo_root)?;
    let shared_seed = SharedConfigSeed {
        worker_key: input.worker_key,
        worker_name,
        transport,
        runtime_target: repo_settings.runtime_target,
        paths,
        ait_web_url: optional_normalized_url(
            sources.value(web_url_names(transport), None).as_deref(),
        ),
    };

    match transport {
        TransportKind::Telegram => resolve_telegram_config(
            shared_seed,
            &sources,
            repo_settings.default_model.as_deref(),
        ),
        TransportKind::Line => resolve_line_config(shared_seed, &sources),
        TransportKind::Discord => resolve_discord_config(shared_seed, &sources),
        TransportKind::Slack => resolve_slack_config(shared_seed, &sources),
    }
}

struct ConfigSources<'a> {
    process_env: &'a BTreeMap<String, String>,
    env_file: &'a BTreeMap<String, String>,
}

impl ConfigSources<'_> {
    fn value(&self, names: &[&str], default: Option<&str>) -> Option<String> {
        for name in names {
            if let Some(value) = clean_map_text(self.process_env.get(*name)) {
                return Some(value);
            }
            if let Some(value) = clean_map_text(self.env_file.get(*name)) {
                return Some(value);
            }
        }
        default.map(str::to_string)
    }
}

struct RepoSettings {
    runtime_target: AgentRuntimeTarget,
    default_model: Option<String>,
}

struct SharedConfigSeed {
    worker_key: String,
    worker_name: String,
    transport: TransportKind,
    runtime_target: AgentRuntimeTarget,
    paths: AgentWorkerRuntimePaths,
    ait_web_url: Option<String>,
}

impl SharedConfigSeed {
    fn finish(self, request_timeout_seconds: Option<f64>) -> AgentSharedWorkerConfig {
        AgentSharedWorkerConfig {
            worker_key: self.worker_key,
            worker_name: self.worker_name,
            transport: self.transport,
            runtime_target: self.runtime_target,
            paths: self.paths,
            ait_web_url: self.ait_web_url,
            request_timeout_seconds,
        }
    }
}

fn resolve_telegram_config(
    shared_seed: SharedConfigSeed,
    sources: &ConfigSources<'_>,
    repo_default_model: Option<&str>,
) -> Result<AgentWorkerRuntimeConfig, String> {
    let token = required_secret(
        sources,
        &["AIT_TELEGRAM_BOT_TOKEN", "BOT_TOKEN"],
        "Telegram bot token",
    )?;
    let username = sources
        .value(&["AIT_TELEGRAM_BOT_USERNAME", "BOT_USERNAME"], Some(""))
        .unwrap_or_default()
        .trim_start_matches('@')
        .to_string();
    let request_timeout_seconds = parse_telegram_timeout_seconds(
        sources
            .value(
                &[
                    "AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS",
                    "AIT_TELEGRAM_TIMEOUT_SECONDS",
                ],
                None,
            )
            .as_deref(),
        None,
        5.0,
    );
    let openai_api_key = sources
        .value(
            &[
                "AIT_TELEGRAM_OPENAI_API_KEY",
                "AIT_OPENAI_API_KEY",
                "OPENAI_API_KEY",
            ],
            None,
        )
        .and_then(normalize_openai_api_key)
        .map(AgentSecret::new);
    let model_fallback = repo_default_model.unwrap_or(DEFAULT_OPENAI_MODEL);
    let openai_model = sources
        .value(
            &[
                "AIT_TELEGRAM_MODEL",
                "AIT_TELEGRAM_OPENAI_MODEL",
                "AIT_MODEL",
                "CODEX_MODEL",
                "OPENAI_MODEL",
            ],
            Some(model_fallback),
        )
        .unwrap_or_else(|| model_fallback.to_string());
    let stt_mode = match sources
        .value(&["AIT_TELEGRAM_STT_MODE"], Some("off"))
        .unwrap_or_else(|| "off".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "off" => TelegramSttMode::Off,
        "local-stt" => TelegramSttMode::LocalStt,
        _ => {
            return Err(
                "unsupported Telegram STT mode; expected one of: local-stt, off".to_string(),
            )
        }
    };
    let service_mode = match sources
        .value(
            &["AIT_TELEGRAM_MODE", "AIT_TELEGRAM_SERVICE_MODE"],
            Some("poll"),
        )
        .unwrap_or_else(|| "poll".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "poll" => TelegramWorkerMode::Poll,
        "webhook" => TelegramWorkerMode::Webhook,
        _ => {
            return Err(
                "unsupported Telegram worker mode; expected one of: poll, webhook".to_string(),
            )
        }
    };
    let config = TelegramWorkerConfig {
        shared: shared_seed.finish(request_timeout_seconds),
        token,
        username,
        service_mode,
        bind_host: sources
            .value(&["AIT_TELEGRAM_BIND_HOST"], Some("127.0.0.1"))
            .unwrap_or_else(|| "127.0.0.1".to_string()),
        bind_port: parse_port(
            sources.value(&["AIT_TELEGRAM_BIND_PORT"], None).as_deref(),
            8090,
        ),
        webhook_path: normalize_http_path(
            sources
                .value(&["AIT_TELEGRAM_WEBHOOK_PATH"], None)
                .as_deref(),
            "/webhook",
        ),
        webhook_secret: optional_secret(sources, &["AIT_TELEGRAM_WEBHOOK_SECRET"]),
        poll_timeout_seconds: parse_positive_int(
            sources
                .value(&["AIT_TELEGRAM_POLL_TIMEOUT_SECONDS"], None)
                .as_deref(),
            45,
            5,
        ) as u64,
        background_sync_enabled: parse_bool(
            sources
                .value(&["AIT_TELEGRAM_BACKGROUND_SYNC_ENABLED"], None)
                .as_deref(),
            false,
        ),
        background_sync_interval_seconds: parse_positive_float(
            sources
                .value(&["AIT_TELEGRAM_BACKGROUND_SYNC_INTERVAL_SECONDS"], None)
                .as_deref(),
            30.0,
            5.0,
        ),
        openai_api_key,
        openai_base_url: normalized_url(
            sources
                .value(
                    &[
                        "AIT_TELEGRAM_OPENAI_BASE_URL",
                        "AIT_OPENAI_BASE_URL",
                        "OPENAI_BASE_URL",
                    ],
                    None,
                )
                .as_deref(),
            DEFAULT_OPENAI_BASE_URL,
        ),
        openai_model,
        openai_reasoning_effort: sources.value(&["AIT_TELEGRAM_REASONING_EFFORT"], Some("low")),
        openai_timeout_seconds: parse_telegram_timeout_seconds(
            sources
                .value(&["AIT_TELEGRAM_OPENAI_TIMEOUT_SECONDS"], None)
                .as_deref(),
            request_timeout_seconds,
            10.0,
        ),
        openai_max_output_tokens: parse_positive_int(
            sources
                .value(&["AIT_TELEGRAM_MAX_OUTPUT_TOKENS"], None)
                .as_deref(),
            700,
            64,
        ) as u64,
        turn_merge_window_seconds: parse_non_negative_float(
            sources
                .value(&["AIT_TELEGRAM_TURN_MERGE_WINDOW_SECONDS"], None)
                .as_deref(),
            0.35,
        ),
        turn_merge_max_messages: parse_positive_int(
            sources
                .value(&["AIT_TELEGRAM_TURN_MERGE_MAX_MESSAGES"], None)
                .as_deref(),
            4,
            1,
        ) as u64,
        decoupled_reply_enabled: parse_bool(
            sources
                .value(&["AIT_TELEGRAM_DECOUPLED_REPLY_ENABLED"], None)
                .as_deref(),
            true,
        ),
        reply_markdown_enabled: parse_bool(
            sources
                .value(
                    &[
                        "AIT_TELEGRAM_REPLY_MARKDOWN_ENABLED",
                        "AIT_TELEGRAM_MARKDOWN_ENABLED",
                    ],
                    None,
                )
                .as_deref(),
            true,
        ),
        owner_bootstrap_enabled: parse_bool(
            sources
                .value(&["AIT_TELEGRAM_OWNER_BOOTSTRAP_ENABLED"], None)
                .as_deref(),
            true,
        ),
        stt_mode,
        stt_model: sources
            .value(
                &["AIT_TELEGRAM_STT_MODEL"],
                Some(DEFAULT_TELEGRAM_STT_MODEL),
            )
            .unwrap_or_else(|| DEFAULT_TELEGRAM_STT_MODEL.to_string()),
        stt_device: sources
            .value(&["AIT_TELEGRAM_STT_DEVICE"], Some("auto"))
            .unwrap_or_else(|| "auto".to_string())
            .to_ascii_lowercase(),
        stt_compute_type: sources.value(&["AIT_TELEGRAM_STT_COMPUTE_TYPE"], None),
        stt_language: sources.value(&["AIT_TELEGRAM_STT_LANGUAGE"], None),
        stt_include_audio_uploads: parse_bool(
            sources
                .value(&["AIT_TELEGRAM_STT_INCLUDE_AUDIO_UPLOADS"], None)
                .as_deref(),
            false,
        ),
        stt_program: sources
            .value(&["AIT_TELEGRAM_STT_PROGRAM"], None)
            .map(PathBuf::from),
        stt_timeout_seconds: parse_positive_float(
            sources
                .value(&["AIT_TELEGRAM_STT_TIMEOUT_SECONDS"], None)
                .as_deref(),
            120.0,
            0.1,
        )
        .min(3_600.0),
        expected_concurrent_workers: parse_optional_positive_int(
            sources
                .value(
                    &[
                        "AIT_AGENT_EXPECTED_CONCURRENT_WORKERS",
                        "AIT_TELEGRAM_AGENT_EXPECTED_CONCURRENT_WORKERS",
                    ],
                    None,
                )
                .as_deref(),
        ),
        event_loop_backend: sources.value(
            &[
                "AIT_AGENT_EVENT_LOOP_BACKEND",
                "AIT_TELEGRAM_AGENT_EVENT_LOOP_BACKEND",
            ],
            None,
        ),
        workers_per_shard: parse_optional_positive_int(
            sources
                .value(
                    &[
                        "AIT_AGENT_WORKERS_PER_SHARD",
                        "AIT_TELEGRAM_AGENT_WORKERS_PER_SHARD",
                    ],
                    None,
                )
                .as_deref(),
        ),
    };
    Ok(AgentWorkerRuntimeConfig::Telegram(config))
}

fn resolve_line_config(
    shared_seed: SharedConfigSeed,
    sources: &ConfigSources<'_>,
) -> Result<AgentWorkerRuntimeConfig, String> {
    let channel_access_token = required_secret(
        sources,
        &["AIT_LINE_CHANNEL_ACCESS_TOKEN", "LINE_CHANNEL_ACCESS_TOKEN"],
        "LINE channel access token",
    )?;
    let channel_secret = required_secret(
        sources,
        &["AIT_LINE_CHANNEL_SECRET", "LINE_CHANNEL_SECRET"],
        "LINE channel secret",
    )?;
    let request_timeout_seconds = parse_timeout_seconds(
        sources
            .value(
                &[
                    "AIT_LINE_REQUEST_TIMEOUT_SECONDS",
                    "AIT_LINE_TIMEOUT_SECONDS",
                ],
                None,
            )
            .as_deref(),
        Some(20.0),
        5.0,
    );
    Ok(AgentWorkerRuntimeConfig::Line(LineWorkerConfig {
        shared: shared_seed.finish(request_timeout_seconds),
        channel_access_token,
        channel_secret,
        api_base_url: normalized_url(
            sources.value(&["AIT_LINE_API_BASE_URL"], None).as_deref(),
            DEFAULT_LINE_API_BASE_URL,
        ),
        bind_host: sources
            .value(&["AIT_LINE_BIND_HOST"], Some("127.0.0.1"))
            .unwrap_or_else(|| "127.0.0.1".to_string()),
        bind_port: parse_port(
            sources.value(&["AIT_LINE_BIND_PORT"], None).as_deref(),
            8091,
        ),
        webhook_path: normalize_http_path(
            sources.value(&["AIT_LINE_WEBHOOK_PATH"], None).as_deref(),
            "/callback",
        ),
    }))
}

fn resolve_discord_config(
    shared_seed: SharedConfigSeed,
    sources: &ConfigSources<'_>,
) -> Result<AgentWorkerRuntimeConfig, String> {
    let application_id = required_secret(
        sources,
        &["AIT_DISCORD_APPLICATION_ID", "DISCORD_APPLICATION_ID"],
        "Discord application id",
    )?;
    let request_timeout_seconds = parse_timeout_seconds(
        sources
            .value(
                &[
                    "AIT_DISCORD_REQUEST_TIMEOUT_SECONDS",
                    "AIT_DISCORD_TIMEOUT_SECONDS",
                ],
                None,
            )
            .as_deref(),
        Some(20.0),
        5.0,
    );
    let default_turn_timeout = request_timeout_seconds.map(|value| value.max(300.0));
    Ok(AgentWorkerRuntimeConfig::Discord(DiscordWorkerConfig {
        shared: shared_seed.finish(request_timeout_seconds),
        application_id,
        public_key: optional_secret(sources, &["AIT_DISCORD_PUBLIC_KEY", "DISCORD_PUBLIC_KEY"]),
        bot_token: optional_secret(sources, &["AIT_DISCORD_BOT_TOKEN", "DISCORD_BOT_TOKEN"]),
        turn_timeout_seconds: parse_timeout_seconds(
            sources
                .value(
                    &[
                        "AIT_DISCORD_TURN_TIMEOUT_SECONDS",
                        "AIT_DISCORD_CODEX_TURN_TIMEOUT_SECONDS",
                        "AIT_CHAT_CODEX_TURN_TIMEOUT_SECONDS",
                    ],
                    None,
                )
                .as_deref(),
            default_turn_timeout,
            5.0,
        ),
        api_base_url: normalized_url(
            sources
                .value(&["AIT_DISCORD_API_BASE_URL"], None)
                .as_deref(),
            DEFAULT_DISCORD_API_BASE_URL,
        ),
        http_user_agent: sources
            .value(
                &["AIT_DISCORD_HTTP_USER_AGENT", "DISCORD_HTTP_USER_AGENT"],
                Some(DEFAULT_DISCORD_HTTP_USER_AGENT),
            )
            .unwrap_or_else(|| DEFAULT_DISCORD_HTTP_USER_AGENT.to_string()),
        bind_host: sources
            .value(&["AIT_DISCORD_BIND_HOST"], Some("127.0.0.1"))
            .unwrap_or_else(|| "127.0.0.1".to_string()),
        bind_port: parse_port(
            sources.value(&["AIT_DISCORD_BIND_PORT"], None).as_deref(),
            8092,
        ),
        interaction_path: normalize_http_path(
            sources
                .value(&["AIT_DISCORD_INTERACTION_PATH"], None)
                .as_deref(),
            "/interactions",
        ),
    }))
}

fn resolve_slack_config(
    shared_seed: SharedConfigSeed,
    sources: &ConfigSources<'_>,
) -> Result<AgentWorkerRuntimeConfig, String> {
    let request_timeout_seconds = parse_timeout_seconds(
        sources
            .value(
                &[
                    "AIT_SLACK_REQUEST_TIMEOUT_SECONDS",
                    "AIT_SLACK_TIMEOUT_SECONDS",
                ],
                None,
            )
            .as_deref(),
        Some(20.0),
        5.0,
    );
    Ok(AgentWorkerRuntimeConfig::Slack(SlackWorkerConfig {
        shared: shared_seed.finish(request_timeout_seconds),
        app_token: optional_secret(sources, &["AIT_SLACK_APP_TOKEN", "SLACK_APP_TOKEN"]),
        signing_secret: optional_secret(
            sources,
            &["AIT_SLACK_SIGNING_SECRET", "SLACK_SIGNING_SECRET"],
        ),
        api_base_url: normalized_url(
            sources.value(&["AIT_SLACK_API_BASE_URL"], None).as_deref(),
            DEFAULT_SLACK_API_BASE_URL,
        ),
        http_user_agent: sources
            .value(
                &["AIT_SLACK_HTTP_USER_AGENT", "SLACK_HTTP_USER_AGENT"],
                Some(DEFAULT_SLACK_HTTP_USER_AGENT),
            )
            .unwrap_or_else(|| DEFAULT_SLACK_HTTP_USER_AGENT.to_string()),
        bind_host: sources
            .value(&["AIT_SLACK_BIND_HOST"], Some("127.0.0.1"))
            .unwrap_or_else(|| "127.0.0.1".to_string()),
        bind_port: parse_port(
            sources.value(&["AIT_SLACK_BIND_PORT"], None).as_deref(),
            8093,
        ),
        command_path: normalize_http_path(
            sources.value(&["AIT_SLACK_COMMAND_PATH"], None).as_deref(),
            "/command",
        ),
        ack_text: sources
            .value(&["AIT_SLACK_ACK_TEXT"], Some(DEFAULT_SLACK_ACK_TEXT))
            .unwrap_or_else(|| DEFAULT_SLACK_ACK_TEXT.to_string()),
        response_type: sources
            .value(
                &["AIT_SLACK_RESPONSE_TYPE"],
                Some(DEFAULT_SLACK_RESPONSE_TYPE),
            )
            .unwrap_or_else(|| DEFAULT_SLACK_RESPONSE_TYPE.to_string()),
    }))
}

fn resolve_repo_settings(repo_root: &Path) -> Result<RepoSettings, String> {
    let config_path = repo_root.join(".ait").join("config.json");
    let raw = match fs::read_to_string(&config_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}".to_string(),
        Err(error) => {
            return Err(format!(
                "failed to read ait-agent repository config '{}': {error}",
                config_path.display()
            ))
        }
    };
    let config = parse_value(&raw, "invalid ait-agent repository config")?;
    let config = config
        .as_object()
        .ok_or_else(|| "ait-agent repository config must be a JSON object".to_string())?;
    let workflow_mode = resolve_workflow_mode(config)?;
    let repo_name = clean_json_text(config.get("repo_name"))
        .or_else(|| {
            repo_root
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "repo".to_string());
    let default_model = clean_json_text(config.get("default_model"));
    if workflow_mode == AgentWorkflowMode::SoloLocal {
        let optional_remote = resolve_configured_remote(config, false)?;
        return Ok(RepoSettings {
            runtime_target: AgentRuntimeTarget {
                mode: AgentRuntimeMode::Local,
                workflow_mode,
                repo_name,
                repo_root: repo_root.to_path_buf(),
                remote_name: optional_remote.as_ref().map(|(name, _)| name.clone()),
                server_url: optional_remote.map(|(_, url)| url),
            },
            default_model,
        });
    }
    let (remote_name, server_url) = resolve_configured_remote(config, true)?
        .ok_or_else(|| "ait-agent remote workflow requires a default remote name".to_string())?;
    Ok(RepoSettings {
        runtime_target: AgentRuntimeTarget {
            mode: AgentRuntimeMode::Remote,
            workflow_mode,
            repo_name,
            repo_root: repo_root.to_path_buf(),
            remote_name: Some(remote_name),
            server_url: Some(server_url),
        },
        default_model,
    })
}

fn resolve_configured_remote(
    config: &ait_core::json_support::JsonMap<String, JsonValue>,
    required: bool,
) -> Result<Option<(String, String)>, String> {
    let Some(remote_name) = clean_json_text(config.get("default_remote")) else {
        if required {
            return Err("ait-agent remote workflow requires a default remote name".to_string());
        }
        return Ok(None);
    };
    let Some(remotes) = config.get("remotes").and_then(JsonValue::as_object) else {
        return if required {
            Err("ait-agent remote workflow requires a remotes map".to_string())
        } else {
            Ok(None)
        };
    };
    let Some(remote) = remotes.get(&remote_name).and_then(JsonValue::as_object) else {
        return if required {
            Err("ait-agent default remote is not configured".to_string())
        } else {
            Ok(None)
        };
    };
    let Some(raw_server_url) = clean_json_text(remote.get("url")) else {
        return if required {
            Err("the default ait remote is missing a server URL".to_string())
        } else {
            Ok(None)
        };
    };
    let server_url = raw_server_url.trim_end_matches('/').to_string();
    if server_url.is_empty() {
        return if required {
            Err("the default ait remote is missing a server URL".to_string())
        } else {
            Ok(None)
        };
    }
    Ok(Some((remote_name, server_url)))
}

fn resolve_workflow_mode(
    config: &ait_core::json_support::JsonMap<String, JsonValue>,
) -> Result<AgentWorkflowMode, String> {
    if let Some(value) = clean_json_text(config.get("workflow_mode")) {
        match value.as_str() {
            "solo_local" => return Ok(AgentWorkflowMode::SoloLocal),
            "solo_remote" => return Ok(AgentWorkflowMode::SoloRemote),
            "team_remote" => return Ok(AgentWorkflowMode::TeamRemote),
            _ => {}
        }
    }
    let workflow_scope = valid_scope(config.get("workflow_default_scope")).unwrap_or("local");
    let task_scope = valid_scope(config.get("task_default_scope")).unwrap_or(workflow_scope);
    let change_scope = valid_scope(config.get("change_default_scope")).unwrap_or(workflow_scope);
    let binding_mode = config
        .get("plan_task_binding")
        .and_then(JsonValue::as_object)
        .and_then(|binding| clean_json_text(binding.get("mode")))
        .filter(|mode| matches!(mode.as_str(), "advisory" | "strict" | "required"))
        .unwrap_or_else(|| "required".to_string());
    match (
        workflow_scope,
        task_scope,
        change_scope,
        binding_mode.as_str(),
    ) {
        ("local", "local", "local", "required") => Ok(AgentWorkflowMode::SoloLocal),
        ("remote", "remote", "remote", "advisory") => Ok(AgentWorkflowMode::SoloRemote),
        ("remote", "remote", "remote", "required") => Ok(AgentWorkflowMode::TeamRemote),
        _ => Err(
            "ait-agent requires a repo workflow preset: solo_local, solo_remote, or team_remote"
                .to_string(),
        ),
    }
}

fn valid_scope(value: Option<&JsonValue>) -> Option<&str> {
    match value.and_then(JsonValue::as_str).map(str::trim) {
        Some("local") => Some("local"),
        Some("remote") => Some("remote"),
        _ => None,
    }
}

fn overlay_manifest_credentials(
    transport: TransportKind,
    worker: &ait_core::json_support::JsonMap<String, JsonValue>,
    process_env: &mut BTreeMap<String, String>,
) {
    match transport {
        TransportKind::Telegram => {
            overlay_aliases(
                worker,
                "token",
                &["AIT_TELEGRAM_BOT_TOKEN", "BOT_TOKEN"],
                process_env,
            );
            overlay_aliases(
                worker,
                "username",
                &["AIT_TELEGRAM_BOT_USERNAME", "BOT_USERNAME"],
                process_env,
            );
            for (field, aliases) in [
                ("mode", &["AIT_TELEGRAM_MODE"][..]),
                ("bind_host", &["AIT_TELEGRAM_BIND_HOST"][..]),
                ("webhook_path", &["AIT_TELEGRAM_WEBHOOK_PATH"][..]),
                ("webhook_secret", &["AIT_TELEGRAM_WEBHOOK_SECRET"][..]),
            ] {
                overlay_aliases(worker, field, aliases, process_env);
            }
            overlay_scalar_aliases(
                worker,
                "bind_port",
                &["AIT_TELEGRAM_BIND_PORT"],
                process_env,
            );
        }
        TransportKind::Line => {
            overlay_aliases(
                worker,
                "token",
                &["AIT_LINE_CHANNEL_ACCESS_TOKEN", "LINE_CHANNEL_ACCESS_TOKEN"],
                process_env,
            );
            overlay_aliases(
                worker,
                "secret",
                &["AIT_LINE_CHANNEL_SECRET", "LINE_CHANNEL_SECRET"],
                process_env,
            );
        }
        TransportKind::Discord => {
            overlay_aliases(
                worker,
                "application_id",
                &["AIT_DISCORD_APPLICATION_ID", "DISCORD_APPLICATION_ID"],
                process_env,
            );
            overlay_aliases(
                worker,
                "public_key",
                &["AIT_DISCORD_PUBLIC_KEY", "DISCORD_PUBLIC_KEY"],
                process_env,
            );
            overlay_aliases(
                worker,
                "bot_token",
                &["AIT_DISCORD_BOT_TOKEN", "DISCORD_BOT_TOKEN"],
                process_env,
            );
        }
        TransportKind::Slack => {
            overlay_aliases(
                worker,
                "app_token",
                &["AIT_SLACK_APP_TOKEN", "SLACK_APP_TOKEN"],
                process_env,
            );
            overlay_aliases(
                worker,
                "signing_secret",
                &["AIT_SLACK_SIGNING_SECRET", "SLACK_SIGNING_SECRET"],
                process_env,
            );
            overlay_aliases(
                worker,
                "api_base_url",
                &["AIT_SLACK_API_BASE_URL"],
                process_env,
            );
            overlay_aliases(
                worker,
                "http_user_agent",
                &["AIT_SLACK_HTTP_USER_AGENT"],
                process_env,
            );
        }
    }
}

fn overlay_aliases(
    worker: &ait_core::json_support::JsonMap<String, JsonValue>,
    field: &str,
    aliases: &[&str],
    process_env: &mut BTreeMap<String, String>,
) {
    let Some(value) = clean_json_text(worker.get(field)) else {
        return;
    };
    for alias in aliases {
        process_env.insert((*alias).to_string(), value.clone());
    }
}

fn overlay_scalar_aliases(
    worker: &ait_core::json_support::JsonMap<String, JsonValue>,
    field: &str,
    aliases: &[&str],
    process_env: &mut BTreeMap<String, String>,
) {
    let Some(value) = worker_scalar_text(worker.get(field)) else {
        return;
    };
    for alias in aliases {
        process_env.insert((*alias).to_string(), value.clone());
    }
}

fn worker_scalar_text(value: Option<&JsonValue>) -> Option<String> {
    match value? {
        JsonValue::String(value) => agent_transport_config_clean_optional_text(Some(value)),
        JsonValue::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn apply_runtime_path_overrides(
    transport: TransportKind,
    repo_root: &Path,
    sources: &ConfigSources<'_>,
    process_env: &BTreeMap<String, String>,
    paths: &mut AgentWorkerRuntimePaths,
) {
    if let Some(value) = sources.value(&[transport_state_path_name(transport)], None) {
        paths.sync_state_path = resolve_path(repo_root, &value, process_env)
            .to_string_lossy()
            .into_owned();
    }
    if let Some(value) = sources.value(&[transport_termination_path_name(transport)], None) {
        paths.termination_context_path = resolve_path(repo_root, &value, process_env)
            .to_string_lossy()
            .into_owned();
    }
}

fn transport_env_path_name(transport: TransportKind) -> &'static str {
    match transport {
        TransportKind::Telegram => "AIT_TELEGRAM_ENV_PATH",
        TransportKind::Line => "AIT_LINE_ENV_PATH",
        TransportKind::Discord => "AIT_DISCORD_ENV_PATH",
        TransportKind::Slack => "AIT_SLACK_ENV_PATH",
    }
}

fn transport_state_path_name(transport: TransportKind) -> &'static str {
    match transport {
        TransportKind::Telegram => "AIT_TELEGRAM_STATE_PATH",
        TransportKind::Line => "AIT_LINE_STATE_PATH",
        TransportKind::Discord => "AIT_DISCORD_STATE_PATH",
        TransportKind::Slack => "AIT_SLACK_STATE_PATH",
    }
}

fn transport_termination_path_name(transport: TransportKind) -> &'static str {
    match transport {
        TransportKind::Telegram => "AIT_TELEGRAM_TERMINATION_CONTEXT_PATH",
        TransportKind::Line => "AIT_LINE_TERMINATION_CONTEXT_PATH",
        TransportKind::Discord => "AIT_DISCORD_TERMINATION_CONTEXT_PATH",
        TransportKind::Slack => "AIT_SLACK_TERMINATION_CONTEXT_PATH",
    }
}

fn web_url_names(transport: TransportKind) -> &'static [&'static str] {
    match transport {
        TransportKind::Telegram => &["AIT_TELEGRAM_WEB_URL", "AIT_WEB_URL"],
        TransportKind::Line => &["AIT_LINE_WEB_URL", "AIT_WEB_URL"],
        TransportKind::Discord => &["AIT_DISCORD_WEB_URL", "AIT_WEB_URL"],
        TransportKind::Slack => &["AIT_SLACK_WEB_URL", "AIT_WEB_URL"],
    }
}

fn required_secret(
    sources: &ConfigSources<'_>,
    names: &[&str],
    label: &str,
) -> Result<AgentSecret, String> {
    sources
        .value(names, None)
        .map(AgentSecret::new)
        .ok_or_else(|| format!("ait-agent worker configuration is missing {label}"))
}

fn optional_secret(sources: &ConfigSources<'_>, names: &[&str]) -> Option<AgentSecret> {
    sources.value(names, None).map(AgentSecret::new)
}

fn clean_json_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .and_then(|value| agent_transport_config_clean_optional_text(Some(value)))
}

fn clean_map_text(value: Option<&String>) -> Option<String> {
    value.and_then(|value| agent_transport_config_clean_optional_text(Some(value)))
}

fn normalized_url(value: Option<&str>, fallback: &str) -> String {
    agent_transport_config_normalize_base_url(value, fallback)
}

fn optional_normalized_url(value: Option<&str>) -> Option<String> {
    let value = agent_transport_config_clean_optional_text(value)?;
    Some(value.trim_end_matches('/').to_string())
}

fn parse_timeout_seconds(value: Option<&str>, fallback: Option<f64>, minimum: f64) -> Option<f64> {
    parse_timeout_seconds_with_policy(
        value,
        fallback,
        minimum,
        DEFAULT_TIMEOUT_DISABLE_TOKENS,
        false,
    )
}

fn parse_telegram_timeout_seconds(
    value: Option<&str>,
    fallback: Option<f64>,
    minimum: f64,
) -> Option<f64> {
    parse_timeout_seconds_with_policy(
        value,
        fallback,
        minimum,
        TELEGRAM_TIMEOUT_DISABLE_TOKENS,
        true,
    )
}

fn parse_timeout_seconds_with_policy(
    value: Option<&str>,
    fallback: Option<f64>,
    minimum: f64,
    disable_tokens: &[&str],
    finite_only: bool,
) -> Option<f64> {
    let raw = value.unwrap_or_default().trim().to_ascii_lowercase();
    if raw.is_empty() {
        return fallback;
    }
    if disable_tokens.contains(&raw.as_str()) {
        return None;
    }
    let Ok(parsed) = raw.parse::<f64>() else {
        return fallback;
    };
    if !parsed.is_finite() {
        if finite_only {
            return None;
        }
        if parsed <= 0.0 {
            return fallback;
        }
        return Some(parsed);
    }
    if parsed <= 0.0 {
        return fallback;
    }
    Some(parsed.max(minimum))
}

fn parse_positive_int(value: Option<&str>, fallback: i64, minimum: i64) -> i64 {
    let Ok(parsed) = value.unwrap_or_default().trim().parse::<i64>() else {
        return fallback;
    };
    if parsed <= 0 {
        fallback
    } else {
        parsed.max(minimum)
    }
}

fn parse_optional_positive_int(value: Option<&str>) -> Option<usize> {
    let raw = value.unwrap_or_default().trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = parse_positive_int(Some(raw), 0, 1);
    usize::try_from(parsed).ok().filter(|value| *value > 0)
}

fn parse_positive_float(value: Option<&str>, fallback: f64, minimum: f64) -> f64 {
    let Ok(parsed) = value.unwrap_or_default().trim().parse::<f64>() else {
        return fallback;
    };
    if !parsed.is_finite() || parsed <= 0.0 {
        fallback
    } else {
        parsed.max(minimum)
    }
}

fn parse_non_negative_float(value: Option<&str>, fallback: f64) -> f64 {
    let Ok(parsed) = value.unwrap_or_default().trim().parse::<f64>() else {
        return fallback;
    };
    if !parsed.is_finite() || parsed < 0.0 {
        fallback
    } else {
        parsed
    }
}

fn parse_bool(value: Option<&str>, default: bool) -> bool {
    match value
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

fn parse_port(value: Option<&str>, fallback: i64) -> i64 {
    parse_positive_int(value, fallback, 1)
}

fn normalize_http_path(value: Option<&str>, fallback: &str) -> String {
    let selected =
        agent_transport_config_clean_optional_text(value).unwrap_or_else(|| fallback.to_string());
    if selected.starts_with('/') {
        selected
    } else {
        format!("/{selected}")
    }
}

fn normalize_openai_api_key(value: String) -> Option<String> {
    let lowered = value.to_ascii_lowercase();
    if PLACEHOLDER_OPENAI_API_KEYS.contains(&lowered.as_str()) {
        None
    } else {
        Some(value)
    }
}

fn resolve_path(repo_root: &Path, value: &str, process_env: &BTreeMap<String, String>) -> PathBuf {
    let path = if value == "~" {
        process_env
            .get("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value))
    } else if let Some(rest) = value.strip_prefix("~/") {
        process_env
            .get("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(value))
    } else {
        PathBuf::from(value)
    };
    if path.is_absolute() {
        path
    } else {
        repo_root.join(path)
    }
}

fn select_env_path(
    repo_root: &Path,
    default_path: &Path,
    value: &str,
    process_env: &BTreeMap<String, String>,
) -> PathBuf {
    let candidate = resolve_path(repo_root, value, process_env);
    if !default_path.exists() {
        return candidate;
    }
    let resolved_root = fs::canonicalize(repo_root).unwrap_or_else(|_| repo_root.to_path_buf());
    let resolved_default =
        fs::canonicalize(default_path).unwrap_or_else(|_| default_path.to_path_buf());
    let resolved_candidate = fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
    let candidate_is_repo_local =
        resolved_candidate != resolved_root && resolved_candidate.starts_with(&resolved_root);
    if resolved_candidate != resolved_default && !candidate_is_repo_local {
        default_path.to_path_buf()
    } else {
        candidate
    }
}

fn parse_agent_env_text(raw: &str) -> BTreeMap<String, String> {
    raw.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            let (key, raw_value) = trimmed.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            let value = raw_value.trim();
            let value = if value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')))
            {
                &value[1..value.len() - 1]
            } else {
                value
            };
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn required_request_text(
    value: Option<&JsonValue>,
    operation: &str,
    field: &str,
) -> Result<String, String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| format!("ait-agent {operation} request requires {field}"))
}

#[cfg(test)]
mod solo_local_optional_remote_tests {
    use std::fs;

    use ait_core::json_support::json;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn solo_local_preserves_an_optional_remote_without_changing_local_runtime_mode() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir(temp.path().join(".ait")).expect("ait dir");
        fs::write(
            temp.path().join(".ait/config.json"),
            json!({
                "repo_name": "fixture",
                "workflow_mode": "solo_local",
                "default_remote": "origin",
                "remotes": {"origin": {"url": "http://127.0.0.1:8088///"}}
            })
            .to_string(),
        )
        .expect("config");

        let settings = resolve_repo_settings(temp.path()).expect("repo settings");

        assert_eq!(settings.runtime_target.mode, AgentRuntimeMode::Local);
        assert_eq!(
            settings.runtime_target.workflow_mode,
            AgentWorkflowMode::SoloLocal
        );
        assert_eq!(
            settings.runtime_target.remote_name.as_deref(),
            Some("origin")
        );
        assert_eq!(
            settings.runtime_target.server_url.as_deref(),
            Some("http://127.0.0.1:8088")
        );
    }

    #[test]
    fn solo_local_without_a_default_remote_remains_a_valid_local_target() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir(temp.path().join(".ait")).expect("ait dir");
        fs::write(
            temp.path().join(".ait/config.json"),
            json!({"repo_name": "fixture", "workflow_mode": "solo_local"}).to_string(),
        )
        .expect("config");

        let settings = resolve_repo_settings(temp.path()).expect("repo settings");

        assert_eq!(settings.runtime_target.mode, AgentRuntimeMode::Local);
        assert!(settings.runtime_target.remote_name.is_none());
        assert!(settings.runtime_target.server_url.is_none());

        fs::write(
            temp.path().join(".ait/config.json"),
            json!({
                "repo_name": "fixture",
                "workflow_mode": "solo_local",
                "default_remote": "origin",
                "remotes": {"origin": {}}
            })
            .to_string(),
        )
        .expect("incomplete optional remote");
        let incomplete = resolve_repo_settings(temp.path()).expect("local fallback settings");
        assert_eq!(incomplete.runtime_target.mode, AgentRuntimeMode::Local);
        assert!(incomplete.runtime_target.remote_name.is_none());
        assert!(incomplete.runtime_target.server_url.is_none());
    }
}
