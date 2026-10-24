use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use ait_core::json_support::{json, JsonValue};
use tempfile::{tempdir, TempDir};

use super::*;

fn repo(config: JsonValue) -> TempDir {
    let temp = tempdir().expect("tempdir");
    fs::create_dir(temp.path().join(".ait")).expect("ait dir");
    fs::write(temp.path().join(".ait/config.json"), config.to_string()).expect("repo config");
    temp
}

fn local_repo() -> TempDir {
    repo(json!({
        "repo_name": "typed-fixture",
        "workflow_mode": "solo_local",
        "default_model": "repo-default-model"
    }))
}

fn env_path(repo_root: &Path, worker_key: &str) -> std::path::PathBuf {
    let transport = worker_key.split_once('/').expect("worker key").0;
    repo_root
        .join(".ait")
        .join("agent-runtime")
        .join(format!("{transport}.env"))
}

fn resolve(
    temp: &TempDir,
    worker_key: &str,
    worker: JsonValue,
    env_text: &str,
    process_env: BTreeMap<String, String>,
) -> Result<AgentWorkerRuntimeConfig, String> {
    let path = env_path(temp.path(), worker_key);
    fs::create_dir_all(path.parent().expect("env parent")).expect("runtime dir");
    fs::write(path, env_text).expect("env file");
    resolve_agent_worker_config(AgentWorkerConfigInput {
        repo_root: temp.path().to_path_buf(),
        worker_key: worker_key.to_string(),
        worker,
        process_env,
    })
}

fn env(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

fn assert_redacted(config: &AgentWorkerRuntimeConfig, secrets: &[&str]) {
    let rendered = config.redacted_json().to_string();
    let debug = format!("{config:?}");
    for secret in secrets {
        assert!(!rendered.contains(secret), "redacted JSON leaked a secret");
        assert!(!debug.contains(secret), "Debug leaked a secret");
    }
}

#[test]
fn transport_config_env_parser_preserves_python_file_syntax() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("worker.env");
    fs::write(
        &path,
        "\n# comment\n A = one \nB='two words'\nC=\"three=parts\"\ninvalid\n=missing\nD=#literal\n",
    )
    .expect("env");

    let values = parse_agent_env_file(&path).expect("parsed env");

    assert_eq!(values.get("A").map(String::as_str), Some("one"));
    assert_eq!(values.get("B").map(String::as_str), Some("two words"));
    assert_eq!(values.get("C").map(String::as_str), Some("three=parts"));
    assert_eq!(values.get("D").map(String::as_str), Some("#literal"));
    assert!(!values.contains_key("invalid"));
}

#[test]
fn env_file_load_contract_classifies_missing_and_preserves_parser_semantics() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("worker.env");
    let missing = agent_env_file_load_json(&json!({"path": path})).expect("missing env");
    assert_eq!(missing["contract"], AGENT_ENV_FILE_LOAD_CONTRACT);
    assert_eq!(missing["status"], "not_found");
    assert_eq!(missing["values"], json!({}));
    assert_eq!(missing["python_file_read_allowed"], false);

    fs::write(
        &path,
        " A = one \nB='two words'\nA=last\ninvalid\n=missing\n",
    )
    .expect("env file");
    let loaded = agent_env_file_load_json(&json!({"path": path})).expect("loaded env");
    assert_eq!(loaded["status"], "loaded");
    assert_eq!(loaded["values"]["A"], "last");
    assert_eq!(loaded["values"]["B"], "two words");

    let directory_path = temp.path().join("not-a-file");
    fs::create_dir(&directory_path).expect("directory");
    let error = agent_env_file_load_json(&json!({"path": directory_path}))
        .expect_err("directory read must fail");
    assert!(error.contains("failed to read ait-agent env file"));
}

#[test]
fn repo_default_model_load_contract_classifies_all_file_states() {
    let temp = tempdir().expect("tempdir");
    let request = json!({"repo_root": temp.path()});
    let missing = agent_repo_default_model_load_json(&request).expect("missing config");
    assert_eq!(missing["contract"], AGENT_REPO_DEFAULT_MODEL_LOAD_CONTRACT);
    assert_eq!(missing["status"], "not_found");
    assert!(missing["default_model"].is_null());
    assert_eq!(missing["python_file_read_allowed"], false);

    let config_dir = temp.path().join(".ait");
    let config_path = config_dir.join("config.json");
    fs::create_dir_all(&config_dir).expect("config dir");
    for (payload, status) in [
        ("{not-json", "invalid_json"),
        ("[]", "invalid_payload"),
        ("{}", "missing_model"),
        (r#"{"default_model": 42}"#, "invalid_model"),
    ] {
        fs::write(&config_path, payload).expect("config state");
        let result = agent_repo_default_model_load_json(&request).expect("classified config");
        assert_eq!(result["status"], status);
        assert!(result["default_model"].is_null());
    }

    fs::write(&config_path, r#"{"default_model": "  repo-model  "}"#).expect("model config");
    let loaded = agent_repo_default_model_load_json(&request).expect("loaded model");
    assert_eq!(loaded["status"], "loaded");
    assert_eq!(loaded["default_model"], "repo-model");

    fs::remove_file(&config_path).expect("remove config");
    fs::create_dir(&config_path).expect("unreadable config path");
    let unreadable = agent_repo_default_model_load_json(&request).expect("unreadable config");
    assert_eq!(unreadable["status"], "unreadable");
    assert!(unreadable["default_model"].is_null());
}

#[test]
fn transport_config_prefers_existing_repo_env_over_cross_repo_override() {
    let temp = local_repo();
    let external = tempdir().expect("external tempdir");
    let external_path = external.path().join("telegram.env");
    fs::write(&external_path, "AIT_TELEGRAM_BOT_TOKEN=external-secret\n").expect("external env");
    let mut process_env = BTreeMap::new();
    process_env.insert(
        "AIT_TELEGRAM_ENV_PATH".to_string(),
        external_path.to_string_lossy().into_owned(),
    );

    let config = resolve(
        &temp,
        "telegram/main",
        json!({"kind": "telegram", "name": "main"}),
        "AIT_TELEGRAM_BOT_TOKEN=repo-secret\n",
        process_env,
    )
    .expect("repo-local env");
    let AgentWorkerRuntimeConfig::Telegram(config) = config else {
        panic!("Telegram variant");
    };

    assert_eq!(config.token.expose(), "repo-secret");
    assert_eq!(
        config.shared.paths.env_path,
        env_path(temp.path(), "telegram/main").to_string_lossy()
    );
}

#[test]
fn transport_config_telegram_is_typed_and_manifest_authoritative() {
    let temp = local_repo();
    let config = resolve(
        &temp,
        "telegram/main",
        json!({
            "kind": "telegram",
            "name": "main",
            "token": "manifest-telegram-token",
            "username": "@ait_bot",
            "openai_model": "manifest-model",
            "request_timeout_seconds": "unlimited",
            "openai_timeout_seconds": "null",
            "poll_timeout_seconds": 2,
            "background_sync_enabled": true,
            "background_sync_interval_seconds": 1,
            "turn_merge_window_seconds": 0,
            "reply_markdown_enabled": false,
            "stt_mode": "local-stt",
            "stt_include_audio_uploads": true,
            "stt_program": "/opt/ait/bin/native-stt",
            "stt_timeout_seconds": 17.5,
            "expected_concurrent_workers": 3,
            "event_loop_backend": "portable_poll",
            "workers_per_shard": 2,
            "sync_state_path": "state/telegram.json"
        }),
        "AIT_TELEGRAM_OPENAI_API_KEY=openai-secret\n",
        env(&[("AIT_TELEGRAM_BOT_TOKEN", "process-telegram-token")]),
    )
    .expect("Telegram config");
    let AgentWorkerRuntimeConfig::Telegram(config) = &config else {
        panic!("Telegram variant");
    };

    assert_eq!(config.token.expose(), "manifest-telegram-token");
    assert_eq!(config.username, "ait_bot");
    assert_eq!(config.service_mode, TelegramWorkerMode::Poll);
    assert_eq!(config.bind_host, "127.0.0.1");
    assert_eq!(config.bind_port, 8090);
    assert_eq!(config.webhook_path, "/webhook");
    assert!(config.webhook_secret.is_none());
    assert_eq!(config.openai_model, "manifest-model");
    assert_eq!(config.shared.request_timeout_seconds, None);
    assert_eq!(config.openai_timeout_seconds, None);
    assert_eq!(config.poll_timeout_seconds, 5);
    assert!(config.background_sync_enabled);
    assert_eq!(config.background_sync_interval_seconds, 5.0);
    assert_eq!(config.turn_merge_window_seconds, 0.0);
    assert!(!config.reply_markdown_enabled);
    assert_eq!(config.stt_mode, TelegramSttMode::LocalStt);
    assert!(config.stt_include_audio_uploads);
    assert_eq!(
        config.stt_program.as_deref(),
        Some(std::path::Path::new("/opt/ait/bin/native-stt"))
    );
    assert_eq!(config.stt_timeout_seconds, 17.5);
    assert_eq!(config.expected_concurrent_workers, Some(3));
    assert_eq!(config.event_loop_backend.as_deref(), Some("portable_poll"));
    assert_eq!(config.workers_per_shard, Some(2));
    assert_eq!(
        config.shared.paths.sync_state_path,
        temp.path().join("state/telegram.json").to_string_lossy()
    );
    assert_redacted(
        &AgentWorkerRuntimeConfig::Telegram(config.clone()),
        &[
            "manifest-telegram-token",
            "process-telegram-token",
            "openai-secret",
        ],
    );
}

#[test]
fn transport_config_telegram_webhook_manifest_overrides_and_redacts_secret() {
    let temp = local_repo();
    let config = resolve(
        &temp,
        "telegram/webhook",
        json!({
            "kind": "telegram",
            "name": "webhook",
            "token": "manifest-telegram-token",
            "mode": "webhook",
            "bind_host": "localhost",
            "bind_port": 8181,
            "webhook_path": "telegram-hook",
            "webhook_secret": "manifest-webhook-secret"
        }),
        "AIT_TELEGRAM_MODE=poll\n\
         AIT_TELEGRAM_BIND_HOST=127.0.0.2\n\
         AIT_TELEGRAM_BIND_PORT=8282\n\
         AIT_TELEGRAM_WEBHOOK_PATH=file-hook\n\
         AIT_TELEGRAM_WEBHOOK_SECRET=file-webhook-secret\n",
        env(&[
            ("AIT_TELEGRAM_MODE", "poll"),
            ("AIT_TELEGRAM_BIND_HOST", "127.0.0.3"),
            ("AIT_TELEGRAM_BIND_PORT", "8383"),
            ("AIT_TELEGRAM_WEBHOOK_PATH", "process-hook"),
            ("AIT_TELEGRAM_WEBHOOK_SECRET", "process-webhook-secret"),
        ]),
    )
    .expect("Telegram webhook config");
    let AgentWorkerRuntimeConfig::Telegram(telegram) = &config else {
        panic!("Telegram variant");
    };

    assert_eq!(telegram.service_mode, TelegramWorkerMode::Webhook);
    assert_eq!(telegram.bind_host, "localhost");
    assert_eq!(telegram.bind_port, 8181);
    assert_eq!(telegram.webhook_path, "/telegram-hook");
    assert_eq!(
        telegram.webhook_secret.as_ref().map(AgentSecret::expose),
        Some("manifest-webhook-secret")
    );
    let redacted = config.redacted_json();
    assert_eq!(redacted["credentials"]["webhook_secret_set"], true);
    assert_eq!(redacted["telegram"]["service_mode"], "webhook");
    assert_eq!(redacted["telegram"]["bind_host"], "localhost");
    assert_eq!(redacted["telegram"]["bind_port"], 8181);
    assert_eq!(redacted["telegram"]["webhook_path"], "/telegram-hook");
    assert_redacted(
        &config,
        &[
            "manifest-telegram-token",
            "manifest-webhook-secret",
            "file-webhook-secret",
            "process-webhook-secret",
        ],
    );
}

#[test]
fn transport_config_telegram_missing_and_malformed_values_fail_or_fallback_safely() {
    let temp = local_repo();
    let missing = resolve(
        &temp,
        "telegram/main",
        json!({"kind": "telegram", "name": "main"}),
        "",
        BTreeMap::new(),
    )
    .expect_err("missing token");
    assert!(missing.contains("Telegram bot token"));

    let malformed = resolve(
        &temp,
        "telegram/main",
        json!({"kind": "telegram", "name": "main", "token": "safe-token"}),
        "AIT_TELEGRAM_BIND_PORT=bad\n\
         AIT_TELEGRAM_POLL_TIMEOUT_SECONDS=bad\n\
         AIT_TELEGRAM_MAX_OUTPUT_TOKENS=-1\n\
         AIT_TELEGRAM_BACKGROUND_SYNC_ENABLED=maybe\n\
         AIT_TELEGRAM_TURN_MERGE_WINDOW_SECONDS=-3\n",
        BTreeMap::new(),
    )
    .expect("fallback config");
    let AgentWorkerRuntimeConfig::Telegram(config) = malformed else {
        panic!("Telegram variant");
    };
    assert_eq!(config.bind_port, 8090);
    assert_eq!(config.poll_timeout_seconds, 45);
    assert_eq!(config.openai_max_output_tokens, 700);
    assert!(!config.background_sync_enabled);
    assert_eq!(config.turn_merge_window_seconds, 0.35);

    let unsupported_mode = resolve(
        &temp,
        "telegram/unsupported",
        json!({
            "kind": "telegram",
            "name": "unsupported",
            "token": "safe-token",
            "mode": "sidecar"
        }),
        "",
        BTreeMap::new(),
    )
    .expect_err("unknown mode must fail");
    assert!(unsupported_mode.contains("expected one of: poll, webhook"));
    assert!(!unsupported_mode.contains("safe-token"));

    let invalid_port = resolve(
        &temp,
        "telegram/invalid-port",
        json!({
            "kind": "telegram",
            "name": "invalid-port",
            "token": "safe-token",
            "bind_port": 70000
        }),
        "",
        BTreeMap::new(),
    )
    .expect("typed port remains available for host validation");
    let AgentWorkerRuntimeConfig::Telegram(invalid_port) = invalid_port else {
        panic!("Telegram variant");
    };
    assert_eq!(invalid_port.bind_port, 70000);
}

#[test]
fn transport_config_line_handles_disabled_timeout_overrides_and_redaction() {
    let temp = local_repo();
    let config = resolve(
        &temp,
        "line/main",
        json!({
            "kind": "line",
            "name": "main",
            "token": "manifest-line-token",
            "secret": "manifest-line-secret",
            "request_timeout_seconds": "none",
            "api_base_url": "https://api.line.example///",
            "bind_port": "bad",
            "webhook_path": "events"
        }),
        "AIT_LINE_REQUEST_TIMEOUT_SECONDS=99\n\
         AIT_LINE_API_BASE_URL=https://ignored.example\n",
        env(&[
            ("AIT_LINE_CHANNEL_ACCESS_TOKEN", "process-line-token"),
            ("AIT_LINE_CHANNEL_SECRET", "process-line-secret"),
        ]),
    )
    .expect("LINE config");
    let AgentWorkerRuntimeConfig::Line(line) = &config else {
        panic!("LINE variant");
    };

    assert_eq!(line.channel_access_token.expose(), "manifest-line-token");
    assert_eq!(line.channel_secret.expose(), "manifest-line-secret");
    assert_eq!(line.shared.request_timeout_seconds, None);
    assert_eq!(line.api_base_url, "https://api.line.example");
    assert_eq!(line.bind_port, 8091);
    assert_eq!(line.webhook_path, "/events");
    assert_redacted(
        &config,
        &[
            "manifest-line-token",
            "manifest-line-secret",
            "process-line-token",
            "process-line-secret",
        ],
    );

    let missing = resolve(
        &temp,
        "line/missing",
        json!({"kind": "line", "name": "missing", "token": "only-token"}),
        "",
        BTreeMap::new(),
    )
    .expect_err("missing secret");
    assert!(missing.contains("LINE channel secret"));
    assert!(!missing.contains("only-token"));
}

#[test]
fn transport_config_discord_handles_remote_runtime_disabled_timeouts_and_redaction() {
    let temp = repo(json!({
        "repo_name": "remote-fixture",
        "workflow_mode": "solo_remote",
        "default_remote": "origin",
        "remotes": {
            "origin": {"url": "https://ait.example.test///"}
        }
    }));
    let config = resolve(
        &temp,
        "discord/main",
        json!({
            "kind": "discord",
            "name": "main",
            "application_id": "manifest-discord-app",
            "public_key": "manifest-discord-public",
            "bot_token": "manifest-discord-bot",
            "request_timeout_seconds": "inf",
            "turn_timeout_seconds": "unlimited",
            "http_user_agent": "manifest-canonical-agent",
            "bind_port": 0,
            "interaction_path": "interact"
        }),
        "AIT_DISCORD_REQUEST_TIMEOUT_SECONDS=31\n\
         AIT_DISCORD_HTTP_USER_AGENT=ignored-file-agent\n",
        env(&[
            ("DISCORD_HTTP_USER_AGENT", "process-legacy-agent"),
            ("AIT_DISCORD_APPLICATION_ID", "process-discord-app"),
        ]),
    )
    .expect("Discord config");
    let AgentWorkerRuntimeConfig::Discord(discord) = &config else {
        panic!("Discord variant");
    };

    assert_eq!(discord.application_id.expose(), "manifest-discord-app");
    assert_eq!(
        discord.public_key.as_ref().map(AgentSecret::expose),
        Some("manifest-discord-public")
    );
    assert_eq!(
        discord.bot_token.as_ref().map(AgentSecret::expose),
        Some("manifest-discord-bot")
    );
    assert_eq!(discord.shared.request_timeout_seconds, None);
    assert_eq!(discord.turn_timeout_seconds, None);
    assert_eq!(discord.http_user_agent, "manifest-canonical-agent");
    assert_eq!(discord.bind_port, 8092);
    assert_eq!(discord.interaction_path, "/interact");
    assert_eq!(discord.shared.runtime_target.mode, AgentRuntimeMode::Remote);
    assert_eq!(
        discord.shared.runtime_target.server_url.as_deref(),
        Some("https://ait.example.test")
    );
    assert_redacted(
        &config,
        &[
            "manifest-discord-app",
            "manifest-discord-public",
            "manifest-discord-bot",
            "process-discord-app",
        ],
    );

    let missing = resolve(
        &temp,
        "discord/missing",
        json!({"kind": "discord", "name": "missing"}),
        "",
        BTreeMap::new(),
    )
    .expect_err("missing application id");
    assert!(missing.contains("Discord application id"));
}

#[test]
fn transport_config_slack_keeps_optional_credentials_typed_and_redacted() {
    let temp = local_repo();
    let config = resolve(
        &temp,
        "slack/main",
        json!({
            "kind": "slack",
            "name": "main",
            "app_token": "manifest-slack-app",
            "signing_secret": "manifest-slack-signing",
            "api_base_url": "https://slack.manifest.example/api/",
            "http_user_agent": "manifest-slack-agent",
            "request_timeout_seconds": "none",
            "bind_port": 70000,
            "command_path": "ait",
            "ack_text": "queued",
            "response_type": "ephemeral"
        }),
        "AIT_SLACK_REQUEST_TIMEOUT_SECONDS=42\n\
         AIT_SLACK_BIND_PORT=9000\n\
         AIT_SLACK_COMMAND_PATH=ignored\n\
         AIT_SLACK_ACK_TEXT=ignored\n\
         AIT_SLACK_RESPONSE_TYPE=ignored\n\
         AIT_SLACK_API_BASE_URL=https://slack.file.example/api\n\
         AIT_SLACK_HTTP_USER_AGENT=file-slack-agent\n",
        env(&[
            ("AIT_SLACK_APP_TOKEN", "process-slack-app"),
            ("AIT_SLACK_SIGNING_SECRET", "process-slack-signing"),
            (
                "AIT_SLACK_API_BASE_URL",
                "https://slack.process.example/api",
            ),
            ("AIT_SLACK_HTTP_USER_AGENT", "process-slack-agent"),
        ]),
    )
    .expect("Slack config");
    let AgentWorkerRuntimeConfig::Slack(slack) = &config else {
        panic!("Slack variant");
    };

    assert_eq!(
        slack.app_token.as_ref().map(AgentSecret::expose),
        Some("manifest-slack-app")
    );
    assert_eq!(
        slack.signing_secret.as_ref().map(AgentSecret::expose),
        Some("manifest-slack-signing")
    );
    assert_eq!(slack.shared.request_timeout_seconds, None);
    assert_eq!(slack.bind_port, 70000);
    assert_eq!(slack.command_path, "/ait");
    assert_eq!(slack.ack_text, "queued");
    assert_eq!(slack.response_type, "ephemeral");
    assert_eq!(slack.api_base_url, "https://slack.manifest.example/api");
    assert_eq!(slack.http_user_agent, "manifest-slack-agent");
    assert_redacted(
        &config,
        &[
            "manifest-slack-app",
            "manifest-slack-signing",
            "process-slack-app",
            "process-slack-signing",
        ],
    );

    let optional = resolve(
        &temp,
        "slack/http",
        json!({"kind": "slack", "name": "http"}),
        "AIT_SLACK_REQUEST_TIMEOUT_SECONDS=null\nAIT_SLACK_BIND_PORT=bad\n",
        BTreeMap::new(),
    )
    .expect("optional Slack credentials");
    let AgentWorkerRuntimeConfig::Slack(optional) = optional else {
        panic!("Slack variant");
    };
    assert!(optional.app_token.is_none());
    assert!(optional.signing_secret.is_none());
    assert_eq!(optional.shared.request_timeout_seconds, Some(20.0));
    assert_eq!(optional.bind_port, 8093);
    assert_eq!(optional.api_base_url, "https://slack.com/api");
    assert_eq!(optional.http_user_agent, "ait-agent-worker/0.1");
}

#[test]
fn transport_config_validates_and_preserves_typed_local_reply_settings() {
    let temp = local_repo();
    let local_reply = json!({
        "program": "/opt/ait/bin/ait-agent-worker",
        "args": ["reply-provider"],
        "timeout_seconds": 45,
        "append_turn_analysis": true,
        "codex_program": "/opt/ait/bin/codex",
        "model": "gpt-5.6",
        "reasoning_effort": "high",
        "sandbox": "workspace-write",
        "turn_timeout_seconds": "none",
    });
    let config = resolve(
        &temp,
        "telegram/main",
        json!({
            "kind": "telegram",
            "name": "main",
            "token": "safe-token",
            "local_reply": local_reply.clone(),
        }),
        "AIT_TELEGRAM_CODEX_MODEL=ignored-file-model\n",
        env(&[("AIT_TELEGRAM_CODEX_MODEL", "ignored-process-model")]),
    )
    .expect("typed local reply");

    assert_eq!(config.shared().local_reply.as_ref(), Some(&local_reply));
    assert_eq!(
        config.redacted_json()["common"]["local_reply_configured"],
        true
    );

    let error = resolve(
        &temp,
        "telegram/invalid",
        json!({
            "kind": "telegram",
            "name": "invalid",
            "token": "safe-token",
            "local_reply": {"unknown_setting": true},
        }),
        "",
        BTreeMap::new(),
    )
    .expect_err("unknown local reply setting must fail closed");
    assert!(error.contains("unsupported field"));
    assert!(!error.contains("safe-token"));
}

#[test]
fn transport_config_legacy_scope_defaults_remain_readable_and_bad_remote_fails_closed() {
    let legacy = repo(json!({"repo_name": "legacy"}));
    let config = resolve(
        &legacy,
        "telegram/main",
        json!({"kind": "telegram", "name": "main", "token": "token"}),
        "",
        BTreeMap::new(),
    )
    .expect("legacy local config");
    assert_eq!(
        config.shared().runtime_target.workflow_mode,
        AgentWorkflowMode::SoloLocal
    );

    let invalid_remote = repo(json!({
        "workflow_mode": "team_remote",
        "default_remote": "origin",
        "remotes": {"origin": {}}
    }));
    let error = resolve(
        &invalid_remote,
        "telegram/main",
        json!({"kind": "telegram", "name": "main", "token": "token"}),
        "",
        BTreeMap::new(),
    )
    .expect_err("remote URL required");
    assert!(error.contains("server URL"));
}
