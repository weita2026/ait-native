use crate::agent_harness::converge_agent_workflow_harness;
use crate::config_surface::{config_set, ConfigSetRequest};
use crate::doctor_surface::doctor_runtime_root;
use crate::init_surface::{init_repo_for_install, InitRequest};
use crate::json_support::{encode_value_pretty_with_newline_error_string, parse_value};
use crate::remote_surface::{remote_add, remote_list, set_default_remote, RemoteAddRequest};
use crate::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::worker_manifest::{
    default_worker_manifest_config_json, upsert_worker_manifest_worker_json,
};
use chrono::Utc;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const DEFAULT_REMOTE_NAME: &str = "origin";
const DEFAULT_WORKER_NAME: &str = "main";
const DEFAULT_LINE: &str = "main";
const DEFAULT_POLICY_PROFILE: &str = "prototype";
const DEFAULT_AUTHOR_MODE: &str = "ai_with_human_review";
const TELEGRAM_TOKEN_ENV: &str = "AIT_TELEGRAM_BOT_TOKEN";
const DISCORD_APPLICATION_ID_ENV: &str = "AIT_DISCORD_APPLICATION_ID";
const DISCORD_BOT_TOKEN_ENV: &str = "AIT_DISCORD_BOT_TOKEN";

#[derive(Clone, Debug)]
pub struct InstallRequest {
    pub cwd: PathBuf,
    pub mode: Option<String>,
    pub attach: Option<String>,
    pub server_setup: Option<String>,
    pub server_url: Option<String>,
    pub remote_name: String,
    pub remote_repo_name: Option<String>,
    pub repo_name: Option<String>,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
    pub initialize: Option<bool>,
    pub sprint: Option<bool>,
    pub worker_name: String,
    pub telegram_token: Option<String>,
    pub telegram_username: Option<String>,
    pub discord_application_id: Option<String>,
    pub discord_bot_token: Option<String>,
    pub dry_run: bool,
    pub json_output: bool,
    pub interactive: bool,
}

impl InstallRequest {
    pub fn from_current_dir() -> Result<Self, String> {
        Ok(Self {
            cwd: env::current_dir().map_err(|err| err.to_string())?,
            mode: None,
            attach: None,
            server_setup: None,
            server_url: None,
            remote_name: DEFAULT_REMOTE_NAME.to_string(),
            remote_repo_name: None,
            repo_name: None,
            user_name: None,
            user_email: None,
            initialize: None,
            sprint: None,
            worker_name: DEFAULT_WORKER_NAME.to_string(),
            telegram_token: None,
            telegram_username: None,
            discord_application_id: None,
            discord_bot_token: None,
            dry_run: false,
            json_output: false,
            interactive: true,
        })
    }
}

pub fn install_from_payload(payload: &JsonValue) -> Result<JsonValue, String> {
    install(&parse_install_request(payload)?)
}

pub fn install(request: &InstallRequest) -> Result<JsonValue, String> {
    let existing_repo = RepoRuntime::discover_from_path(&request.cwd).ok();
    let mut resolved_request = request.clone();
    if existing_repo.is_none() {
        if request.initialize == Some(false) {
            return Err(
                "No `ait` repository found in the current directory or its parents.".to_string(),
            );
        }
        if request.initialize.is_none() && !request.json_output && request.interactive {
            let should_initialize = prompt_confirm(
                "No `ait` repository found. Initialize the current directory now?",
                true,
            )?;
            if !should_initialize {
                return Err("Install aborted before repository initialization.".to_string());
            }
        }
        resolved_request.initialize = Some(true);
    }
    let mode = resolve_mode(request.mode.as_deref(), existing_repo.as_ref(), request)?;
    let sprint_enabled = resolve_sprint(request.sprint, existing_repo.as_ref(), request)?;
    let attach = resolve_attach(request.attach.as_deref(), request)?;
    let server_setup = resolve_server_setup(request.server_setup.as_deref(), &mode, request)?;
    let prospective_repo_root = existing_repo
        .as_ref()
        .map(RepoRuntime::authoritative_repo_root)
        .unwrap_or_else(|| {
            request
                .cwd
                .canonicalize()
                .unwrap_or_else(|_| request.cwd.clone())
        });
    let before_workers = load_agent_workers(&prospective_repo_root)?;
    if server_setup == "connect" {
        resolved_request.server_url = Some(
            require_text(request.server_url.as_deref(), "ait-server URL", request)?
                .trim_end_matches('/')
                .to_string(),
        );
    }
    resolve_transport_request(&mut resolved_request, &attach, &before_workers)?;

    let (repo, repo_info) = ensure_repo_context(&resolved_request, sprint_enabled)?;
    let repo_root = repo_info
        .get("repo_root")
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| resolved_request.cwd.clone());
    let identity = resolve_identity(repo.as_ref(), &resolved_request)?;

    // Remote registration has its own admission and may provide a generated CI
    // template. Run it before changing workflow settings so a blocked connect
    // leaves an existing repository's mode and sprint choices untouched.
    let server = configure_server_setup(
        repo.as_ref(),
        &repo_info,
        &mode,
        &server_setup,
        &resolved_request,
    )?;
    let repo_initialized = repo_info
        .get("repo_initialized")
        .and_then(JsonValue::as_bool)
        == Some(true);
    let fresh_install = repo_info
        .get("state")
        .and_then(JsonValue::as_str)
        .is_some_and(|state| state == "initialized_repo" || state == "missing_repo");
    let configure_mode = repo_initialized || request.mode.is_some();
    let configure_sprint = repo_initialized || request.sprint.is_some();

    let config_action = match repo.as_ref() {
        Some(repo) => apply_install_config(
            repo,
            &mode,
            sprint_enabled,
            configure_mode,
            configure_sprint,
            &identity,
            resolved_request.dry_run,
        )?,
        None => preview_install_config_action(&mode, sprint_enabled),
    };
    let effective_mode = match repo.as_ref() {
        Some(repo) if !resolved_request.dry_run => RepoRuntime::discover_from_path(&repo.root)?
            .effective_workflow_mode()
            .to_string(),
        _ => mode.clone(),
    };
    let transport_actions =
        configure_transports(&repo_root, &attach, &before_workers, &resolved_request)?;
    let transport_planned = transport_actions
        .as_array()
        .is_some_and(|actions| !actions.is_empty());
    let worker_manifest =
        secure_worker_manifest(&repo_root, resolved_request.dry_run, transport_planned)?;
    let agent_harness = if resolved_request.dry_run {
        json!({
            "status": "preview",
            "artifact_path": "AGENTS.md",
        })
    } else {
        let refreshed = RepoRuntime::discover_from_path(&repo_root)?;
        converge_agent_workflow_harness(&refreshed)?
    };
    let runtime_root = classify_runtime_root(&repo_root);
    let default_line = repo
        .as_ref()
        .map(RepoRuntime::default_line_name)
        .unwrap_or_else(|| DEFAULT_LINE.to_string());
    let starter_sprint =
        starter_sprint_payload(&repo_info, &effective_mode, sprint_enabled, &default_line);
    let mut payload = json!({
        "repository": repo_info,
        "mode": {
            "requested_mode": mode,
            "effective_mode": effective_mode,
            "action": config_action,
            "source": install_choice_source(request.mode.is_some(), existing_repo.is_some(), request),
        },
        "identity": {
            "user_name": identity.user_name.clone(),
            "user_email": identity.user_email.clone(),
            "action": identity.action.as_str(),
        },
        "sprint": {
            "enabled": sprint_enabled,
            "value": if sprint_enabled { "on" } else { "off" },
            "plan_task_binding_mode": if sprint_enabled { "required" } else { "off" },
            "source": install_choice_source(request.sprint.is_some(), existing_repo.is_some(), request),
        },
        "attach_choice": attach,
        "server": server,
        "runtime_root": runtime_root,
        "transport_actions": transport_actions,
        "worker_manifest": worker_manifest,
        "starter_sprint": starter_sprint,
        "agent_harness": agent_harness,
        "dry_run": resolved_request.dry_run,
    });
    let server_ref = payload
        .get("server")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let transport_ref = payload
        .get("transport_actions")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    replace_object_field(
        &mut payload,
        "next_steps",
        JsonValue::Array(
            mode_next_steps(
                &mode,
                sprint_enabled,
                fresh_install,
                &transport_ref,
                &server_ref,
            )
            .into_iter()
            .map(JsonValue::String)
            .collect(),
        ),
    )?;
    if !resolved_request.json_output {
        let rendered_text = render_install_text(&payload);
        replace_object_field(
            &mut payload,
            "rendered_text",
            JsonValue::String(rendered_text),
        )?;
    }
    Ok(payload)
}

pub fn render_install_text(payload: &JsonValue) -> String {
    let mut lines = vec!["ait install summary".to_string(), String::new()];
    let repo = payload.get("repository").and_then(JsonValue::as_object);
    let mode = payload.get("mode").and_then(JsonValue::as_object);
    lines.push(format!(
        "- repo root: {}",
        object_string(repo, "repo_root").unwrap_or_else(|| "unknown".to_string())
    ));
    lines.push(format!(
        "- repo action: {}",
        object_string(repo, "action").unwrap_or_else(|| "unknown".to_string())
    ));
    lines.push(format!(
        "- workflow mode: {}",
        object_string(mode, "effective_mode").unwrap_or_else(|| "unknown".to_string())
    ));
    if repo
        .and_then(|value| value.get("repo_initialized"))
        .and_then(JsonValue::as_bool)
        == Some(true)
    {
        lines.push("- repository was initialized during this run".to_string());
    }
    if let Some(sprint) = payload.get("sprint").and_then(JsonValue::as_object) {
        let value = object_string(Some(sprint), "value").unwrap_or_else(|| "unknown".to_string());
        let binding = object_string(Some(sprint), "plan_task_binding_mode")
            .unwrap_or_else(|| "unknown".to_string());
        lines.push(format!("- sprint: {value} ({binding})"));
    }
    if let Some(identity) = payload.get("identity").and_then(JsonValue::as_object) {
        let label = match (
            object_string(Some(identity), "user_name"),
            object_string(Some(identity), "user_email"),
        ) {
            (Some(name), Some(email)) => format!("{name} <{email}>"),
            (Some(name), None) => name,
            (None, Some(email)) => email,
            (None, None) => "not configured".to_string(),
        };
        let action =
            object_string(Some(identity), "action").unwrap_or_else(|| "unknown".to_string());
        if label != "not configured" || action != "unchanged" {
            lines.push(format!("- identity: {label} ({action})"));
        }
    }
    let runtime_root = payload.get("runtime_root").and_then(JsonValue::as_object);
    lines.push(format!(
        "- runtime-root classification: {}",
        object_string(runtime_root, "classification").unwrap_or_else(|| "unknown".to_string())
    ));
    if let Some(server) = payload.get("server").and_then(JsonValue::as_object) {
        lines.push(format!(
            "- ait-server setup: {} ({}, {})",
            object_string(Some(server), "choice").unwrap_or_else(|| "unknown".to_string()),
            object_string(Some(server), "action").unwrap_or_else(|| "unknown".to_string()),
            object_string(Some(server), "classification").unwrap_or_else(|| "unknown".to_string())
        ));
    }
    let transport_actions = payload
        .get("transport_actions")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if !transport_actions.is_empty() {
        lines.push(String::new());
        lines.push("Transport actions".to_string());
        for item in &transport_actions {
            let row = item.as_object();
            lines.push(format!(
                "- {}/{}: {}",
                object_string(row, "kind").unwrap_or_else(|| "unknown".to_string()),
                object_string(row, "name").unwrap_or_else(|| "unknown".to_string()),
                object_string(row, "action").unwrap_or_else(|| "unknown".to_string())
            ));
        }
    }
    if let Some(worker_manifest) = payload
        .get("worker_manifest")
        .and_then(JsonValue::as_object)
    {
        if !transport_actions.is_empty()
            || object_string(Some(worker_manifest), "security_action").as_deref()
                != Some("not_present")
        {
            lines.push(format!(
                "- worker credential storage: plaintext JSON at {} ({})",
                object_string(Some(worker_manifest), "path")
                    .unwrap_or_else(|| ".ait/agent-workers.json".to_string()),
                object_string(Some(worker_manifest), "security_action")
                    .unwrap_or_else(|| "unknown".to_string())
            ));
        }
    }
    if let Some(starter) = payload.get("starter_sprint").and_then(JsonValue::as_object) {
        lines.push(String::new());
        lines.push(format!(
            "Starter sprint card: {}",
            object_string(Some(starter), "path")
                .unwrap_or_else(|| "docs/sprints/first_change.md".to_string())
        ));
        if let Some(template) = object_string(Some(starter), "template") {
            lines.push(template.trim_end().to_string());
        }
        if let Some(command) = object_string(Some(starter), "task_start_command") {
            lines.push(String::new());
            lines.push(format!("Then run: {command}"));
        }
    }
    lines.push(String::new());
    lines.push("Next steps".to_string());
    for step in payload
        .get("next_steps")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
    {
        lines.push(format!("- {step}"));
    }
    lines.join("\n")
}

fn parse_install_request(payload: &JsonValue) -> Result<InstallRequest, String> {
    let object = payload
        .as_object()
        .ok_or_else(|| "install payload must decode to an object.".to_string())?;
    let cwd = optional_string_field(object, "cwd")?
        .map(PathBuf::from)
        .unwrap_or(env::current_dir().map_err(|err| err.to_string())?);
    let json_output = bool_field(object, "json_output")? || bool_field(object, "json")?;
    let interactive = optional_bool_field(object, "interactive")?.unwrap_or(!json_output);
    Ok(InstallRequest {
        cwd,
        mode: optional_string_field(object, "mode")?,
        attach: optional_string_field(object, "attach")?,
        server_setup: optional_string_field(object, "server_setup")?,
        server_url: optional_string_field(object, "server_url")?,
        remote_name: optional_string_field(object, "remote_name")?
            .and_then(normalize_text)
            .unwrap_or_else(|| DEFAULT_REMOTE_NAME.to_string()),
        remote_repo_name: optional_string_field(object, "remote_repo_name")?
            .and_then(normalize_text),
        repo_name: optional_string_field(object, "repo_name")?
            .or(optional_string_field(object, "name")?)
            .and_then(normalize_text),
        user_name: optional_string_field(object, "user_name")?.and_then(normalize_text),
        user_email: optional_string_field(object, "user_email")?.and_then(normalize_text),
        initialize: optional_bool_field(object, "initialize")?
            .or(optional_bool_field(object, "init")?),
        sprint: optional_bool_field(object, "sprint")?,
        worker_name: optional_string_field(object, "worker_name")?
            .and_then(normalize_text)
            .unwrap_or_else(|| DEFAULT_WORKER_NAME.to_string()),
        telegram_token: optional_string_field(object, "telegram_token")?.and_then(normalize_text),
        telegram_username: optional_string_field(object, "telegram_username")?
            .and_then(normalize_text),
        discord_application_id: optional_string_field(object, "discord_application_id")?
            .and_then(normalize_text),
        discord_bot_token: optional_string_field(object, "discord_bot_token")?
            .and_then(normalize_text),
        dry_run: bool_field(object, "dry_run")?,
        json_output,
        interactive,
    })
}

fn resolve_mode(
    value: Option<&str>,
    existing_repo: Option<&RepoRuntime>,
    request: &InstallRequest,
) -> Result<String, String> {
    if let Some(value) = value {
        return normalize_mode(value).ok_or_else(|| {
            "`--mode` must be `local`, `remote`, `solo_local`, or `solo_remote`.".to_string()
        });
    }
    if let Some(repo) = existing_repo {
        return Ok(repo.effective_workflow_mode());
    }
    if request.json_output || !request.interactive {
        return Ok("solo_local".to_string());
    }
    let choice = prompt_choice(
        "Choose workflow mode: local or remote",
        "local",
        &["local", "remote"],
    )?;
    normalize_mode(&choice).ok_or_else(|| "Invalid workflow mode.".to_string())
}

fn normalize_mode(value: &str) -> Option<String> {
    match value.trim().to_lowercase().as_str() {
        "local" | "solo_local" => Some("solo_local".to_string()),
        "remote" | "solo_remote" => Some("solo_remote".to_string()),
        _ => None,
    }
}

fn resolve_sprint(
    value: Option<bool>,
    existing_repo: Option<&RepoRuntime>,
    request: &InstallRequest,
) -> Result<bool, String> {
    if let Some(value) = value {
        return Ok(value);
    }
    if let Some(repo) = existing_repo {
        return Ok(repo.sprint_enabled());
    }
    if request.json_output || !request.interactive {
        return Ok(true);
    }
    prompt_confirm("Enable sprint plan/task binding?", true)
}

fn resolve_attach(value: Option<&str>, request: &InstallRequest) -> Result<String, String> {
    if let Some(value) = value {
        return normalize_attach(value).ok_or_else(|| {
            "`--attach` must be `none`, `telegram`, `discord`, or `both`.".to_string()
        });
    }
    if request.json_output || !request.interactive {
        return Ok("none".to_string());
    }
    prompt_choice(
        "Choose transport attach: none, telegram, discord, or both",
        "none",
        &["none", "telegram", "discord", "both"],
    )
}

fn normalize_attach(value: &str) -> Option<String> {
    match value.trim().to_lowercase().as_str() {
        "none" | "telegram" | "discord" | "both" => Some(value.trim().to_lowercase()),
        _ => None,
    }
}

fn resolve_server_setup(
    value: Option<&str>,
    mode: &str,
    request: &InstallRequest,
) -> Result<String, String> {
    if let Some(value) = value {
        let normalized = normalize_server_setup(value).ok_or_else(|| {
            "`--server-setup` must be `skip`, `connect`, or `deploy`.".to_string()
        })?;
        if !is_remote_mode(mode) && normalized != "skip" {
            return Err(
                "`--server-setup` can only be `connect` or `deploy` when `--mode` is remote."
                    .to_string(),
            );
        }
        return Ok(normalized);
    }
    if !is_remote_mode(mode) || request.json_output || !request.interactive {
        return Ok("skip".to_string());
    }
    prompt_choice(
        "Choose ait-server setup: skip, connect, or deploy",
        "skip",
        &["skip", "connect", "deploy"],
    )
}

fn is_remote_mode(mode: &str) -> bool {
    matches!(mode, "solo_remote" | "team_remote")
}

fn install_choice_source(
    explicitly_requested: bool,
    existing_repo: bool,
    request: &InstallRequest,
) -> &'static str {
    if explicitly_requested {
        "request"
    } else if existing_repo {
        "existing_repository"
    } else if request.interactive && !request.json_output {
        "interactive"
    } else {
        "fresh_repository_default"
    }
}

fn normalize_server_setup(value: &str) -> Option<String> {
    match value.trim().to_lowercase().as_str() {
        "skip" | "none" | "no" => Some("skip".to_string()),
        "connect" | "existing" => Some("connect".to_string()),
        "deploy" | "prepare" | "self-hosted" | "self_hosted" => Some("deploy".to_string()),
        _ => None,
    }
}

fn ensure_repo_context(
    request: &InstallRequest,
    sprint_enabled: bool,
) -> Result<(Option<RepoRuntime>, JsonValue), String> {
    if let Ok(repo) = RepoRuntime::discover_from_path(&request.cwd) {
        let repo_name = repo.repo_name();
        let repo_root = repo.authoritative_repo_root();
        return Ok((
            Some(repo),
            json!({
                "state": "existing_repo",
                "repo_root": repo_root.to_string_lossy().to_string(),
                "repo_name": repo_name,
                "repo_initialized": false,
                "action": "unchanged",
            }),
        ));
    }

    if request.initialize == Some(false) {
        return Err(
            "No `ait` repository found in the current directory or its parents.".to_string(),
        );
    }
    let mut should_init = request.initialize.unwrap_or(true);
    if request.initialize.is_none() && !request.json_output && request.interactive {
        should_init = prompt_confirm(
            "No `ait` repository found. Initialize the current directory now?",
            true,
        )?;
    }
    if !should_init {
        return Err("Install aborted before repository initialization.".to_string());
    }
    let cwd = request
        .cwd
        .canonicalize()
        .unwrap_or_else(|_| request.cwd.clone());
    if request.dry_run {
        let repo_name = request
            .repo_name
            .clone()
            .or_else(|| {
                cwd.file_name()
                    .and_then(|value| value.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "repo".to_string());
        return Ok((
            None,
            json!({
                "state": "missing_repo",
                "repo_root": cwd.to_string_lossy().to_string(),
                "repo_initialized": false,
                "action": "would_create",
                "repo_name": repo_name,
            }),
        ));
    }

    let payload = init_repo_for_install(
        &InitRequest {
            root: cwd.clone(),
            name: request.repo_name.clone(),
            default_line: DEFAULT_LINE.to_string(),
            policy_profile: DEFAULT_POLICY_PROFILE.to_string(),
            default_author_mode: DEFAULT_AUTHOR_MODE.to_string(),
            default_model: None,
            repair_existing: false,
        },
        sprint_enabled,
    )?;
    let repo = RepoRuntime::discover_from_path(&cwd)?;
    Ok((
        Some(repo),
        json!({
            "state": "initialized_repo",
            "repo_root": payload.get("repo_root").cloned().unwrap_or_else(|| JsonValue::String(cwd.to_string_lossy().to_string())),
            "repo_name": payload.get("repo_name").cloned().unwrap_or_else(|| JsonValue::String(cwd.file_name().and_then(|value| value.to_str()).unwrap_or("repo").to_string())),
            "repo_initialized": true,
            "action": "created",
        }),
    ))
}

#[derive(Clone, Debug)]
struct InstallIdentity {
    user_name: Option<String>,
    user_email: Option<String>,
    action: String,
}

fn resolve_identity(
    repo: Option<&RepoRuntime>,
    request: &InstallRequest,
) -> Result<InstallIdentity, String> {
    let existing_name = repo.and_then(|repo| object_string(Some(&repo.config), "user_name"));
    let existing_email = repo.and_then(|repo| object_string(Some(&repo.config), "user_email"));
    let user_name = resolve_identity_field(
        request.user_name.as_deref(),
        "User name (optional)",
        existing_name.as_deref(),
        request,
    )?;
    let user_email = resolve_identity_field(
        request.user_email.as_deref(),
        "User email (optional)",
        existing_email.as_deref(),
        request,
    )?;
    let action = identity_action(
        repo,
        user_name.as_deref(),
        user_email.as_deref(),
        request.dry_run,
    );
    Ok(InstallIdentity {
        user_name,
        user_email,
        action,
    })
}

fn resolve_identity_field(
    value: Option<&str>,
    prompt: &str,
    default: Option<&str>,
    request: &InstallRequest,
) -> Result<Option<String>, String> {
    if let Some(value) = value {
        return Ok(normalize_text(value.to_string()));
    }
    if request.json_output || !request.interactive {
        return Ok(None);
    }
    prompt_line(prompt, Some(default.unwrap_or(""))).map(normalize_text)
}

fn identity_action(
    repo: Option<&RepoRuntime>,
    user_name: Option<&str>,
    user_email: Option<&str>,
    dry_run: bool,
) -> String {
    if user_name.is_none() && user_email.is_none() {
        return "unchanged".to_string();
    }
    let Some(repo) = repo else {
        return "would_configure_after_init".to_string();
    };
    if identity_config_matches(repo, user_name, user_email) {
        "unchanged".to_string()
    } else if dry_run {
        "would_update".to_string()
    } else {
        "updated".to_string()
    }
}

fn apply_install_config(
    repo: &RepoRuntime,
    mode: &str,
    sprint_enabled: bool,
    configure_mode: bool,
    configure_sprint: bool,
    identity: &InstallIdentity,
    dry_run: bool,
) -> Result<String, String> {
    let persist_sprint = configure_mode || configure_sprint;
    let workflow_matches = !configure_mode || workflow_config_matches(repo, mode);
    let sprint_matches = !persist_sprint || sprint_config_matches(repo, sprint_enabled);
    let identity_matches = identity_config_matches(
        repo,
        identity.user_name.as_deref(),
        identity.user_email.as_deref(),
    );
    if dry_run {
        return Ok(if workflow_matches && sprint_matches {
            "would_unchanged".to_string()
        } else {
            "would_update".to_string()
        });
    }
    if workflow_matches && sprint_matches && identity_matches {
        return Ok("unchanged".to_string());
    }
    config_set(
        repo,
        &ConfigSetRequest {
            workflow_mode: configure_mode.then(|| mode.to_string()),
            sprint: persist_sprint.then(|| if sprint_enabled { "on" } else { "off" }.to_string()),
            user_name: identity.user_name.clone(),
            user_email: identity.user_email.clone(),
            ..ConfigSetRequest::default()
        },
    )?;
    Ok(if workflow_matches && sprint_matches {
        "unchanged".to_string()
    } else {
        "updated".to_string()
    })
}

fn preview_install_config_action(_mode: &str, _sprint_enabled: bool) -> String {
    "would_configure_after_init".to_string()
}

fn workflow_config_matches(repo: &RepoRuntime, mode: &str) -> bool {
    repo.config.get("workflow_mode").and_then(JsonValue::as_str) == Some(mode)
        && repo.effective_workflow_mode() == mode
}

fn sprint_config_matches(repo: &RepoRuntime, sprint_enabled: bool) -> bool {
    let sprint = if sprint_enabled { "on" } else { "off" };
    let binding = if sprint_enabled { "required" } else { "off" };
    repo.config.get("sprint").and_then(JsonValue::as_str) == Some(sprint)
        && repo
            .config
            .get("plan_task_binding")
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("mode"))
            .and_then(JsonValue::as_str)
            == Some(binding)
}

fn identity_config_matches(
    repo: &RepoRuntime,
    user_name: Option<&str>,
    user_email: Option<&str>,
) -> bool {
    let name_matches = user_name.is_none_or(|value| {
        repo.config
            .get("user_name")
            .and_then(JsonValue::as_str)
            .is_some_and(|existing| existing == value)
    });
    let email_matches = user_email.is_none_or(|value| {
        repo.config
            .get("user_email")
            .and_then(JsonValue::as_str)
            .is_some_and(|existing| existing == value)
    });
    name_matches && email_matches
}

fn configure_server_setup(
    repo: Option<&RepoRuntime>,
    repo_info: &JsonValue,
    mode: &str,
    server_setup: &str,
    request: &InstallRequest,
) -> Result<JsonValue, String> {
    if !is_remote_mode(mode) {
        return Ok(json!({
            "choice": "skip",
            "action": "not_applicable",
            "classification": "not_applicable",
            "remote_name": JsonValue::Null,
            "server_url": JsonValue::Null,
            "next_steps": [],
        }));
    }
    if server_setup == "skip" {
        if let Some(repo) = repo {
            let preserved_remote_name = repo
                .default_remote_name()
                .unwrap_or_else(|| request.remote_name.clone());
            if let Some(row) = find_remote_row(repo, &preserved_remote_name)? {
                return Ok(json!({
                    "choice": "skip",
                    "action": "existing_remote_preserved",
                    "classification": "configured_unverified",
                    "remote_name": preserved_remote_name,
                    "server_url": row.get("url").cloned().unwrap_or(JsonValue::Null),
                    "repo_name": row.get("repo_name").cloned().unwrap_or(JsonValue::Null),
                    "next_steps": [
                        format!("Verify the configured ait-server with `ait queue summary --remote {}` before starting remote-backed work.", preserved_remote_name),
                    ],
                }));
            }
        }
        return Ok(json!({
            "choice": "skip",
            "action": "skipped",
            "classification": "installed_but_not_configured",
            "remote_name": JsonValue::Null,
            "server_url": JsonValue::Null,
            "next_steps": [
                "Connect an existing ait-server later with `ait remote add origin <url> --repo-name <repo-name> --default`.",
                "Or rerun `ait install --mode remote --server-setup connect --server-url <url>`.",
            ],
        }));
    }
    if server_setup == "deploy" {
        return Ok(json!({
            "choice": "deploy",
            "action": "guidance_only",
            "classification": "installed_but_not_configured",
            "remote_name": request.remote_name.as_str(),
            "server_url": JsonValue::Null,
            "next_steps": [
                "Run `ait doctor runtime-root --json` before placing local ait-server data.",
                "Initialize Binary v0 server authority with `ait-server init`, then verify it with `ait-server probe --defer-ci-admission`.",
                "Start the PostgreSQL-free server with `ait-server run --init-if-missing --defer-ci-admission`, then rerun `ait install --mode remote --server-setup connect --server-url <url>`.",
            ],
        }));
    }

    let server_url = require_text(request.server_url.as_deref(), "ait-server URL", request)?
        .trim_end_matches('/')
        .to_string();
    let desired_repo_name = request
        .remote_repo_name
        .clone()
        .or_else(|| {
            repo_info
                .get("repo_name")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
        .or_else(|| repo.map(|repo| repo.repo_name()));

    let Some(repo) = repo else {
        return Ok(json!({
            "choice": "connect",
            "action": if request.dry_run { "would_configure_after_init" } else { "blocked_missing_repo" },
            "classification": "installed_but_not_configured",
            "remote_name": request.remote_name.as_str(),
            "server_url": server_url,
            "repo_name": desired_repo_name,
            "next_steps": [
                "Initialize this directory as an ait repository before writing remote server config.",
            ],
        }));
    };

    let existing_row = find_remote_row(repo, &request.remote_name)?;
    let default_remote_before = repo.default_remote_name();
    if let Some(row) = existing_row.as_ref() {
        if !existing_remote_matches(row, &server_url, desired_repo_name.as_deref()) {
            let message = format!(
                "Remote `{}` already points somewhere else; choose a different --remote-name or reconcile the existing remote manually.",
                request.remote_name
            );
            if !request.dry_run {
                return Err(message);
            }
            return Ok(json!({
                "choice": "connect",
                "action": "blocked_existing_remote_mismatch",
                "classification": "configuration_conflict",
                "remote_name": request.remote_name.as_str(),
                "server_url": server_url,
                "repo_name": desired_repo_name,
                "existing_remote": {
                    "url": row.get("url").cloned().unwrap_or(JsonValue::Null),
                    "repo_name": row.get("repo_name").cloned().unwrap_or(JsonValue::Null),
                },
                "next_steps": [
                    message,
                ],
            }));
        }
    }

    let action = if existing_row.is_none() {
        if request.dry_run {
            "would_create".to_string()
        } else {
            remote_add(
                repo,
                &RemoteAddRequest {
                    name: request.remote_name.clone(),
                    url: server_url.clone(),
                    repo_name: desired_repo_name.clone(),
                    make_default: true,
                    discard_export: false,
                },
            )?;
            "created".to_string()
        }
    } else if default_remote_before.as_deref() == Some(request.remote_name.as_str()) {
        "unchanged".to_string()
    } else if request.dry_run {
        "would_update_default".to_string()
    } else {
        set_default_remote(repo, &request.remote_name)?;
        "default_updated".to_string()
    };

    Ok(json!({
        "choice": "connect",
        "action": action,
        "classification": "configured_unverified",
        "remote_name": request.remote_name.as_str(),
        "server_url": server_url,
        "repo_name": desired_repo_name,
        "next_steps": [
            format!("Verify the configured ait-server with `ait queue summary --remote {}` before starting remote-backed work.", request.remote_name),
            format!("Publish Markdown lineage with `ait plan sync <file-or-dir> --remote {}` when the plan should become shared.", request.remote_name),
        ],
    }))
}

fn find_remote_row(repo: &RepoRuntime, name: &str) -> Result<Option<JsonValue>, String> {
    Ok(remote_list(repo)?
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| {
            row.get("name")
                .and_then(JsonValue::as_str)
                .is_some_and(|value| value == name)
        })
        .cloned())
}

fn resolve_transport_request(
    resolved: &mut InstallRequest,
    attach: &str,
    before_workers: &JsonMap<String, JsonValue>,
) -> Result<(), String> {
    if attach == "telegram" || attach == "both" {
        let key = format!("telegram/{}", resolved.worker_name);
        let before = before_workers.get(&key).and_then(JsonValue::as_object);
        resolved.telegram_token = Some(resolve_secret(
            resolved.telegram_token.as_deref(),
            TELEGRAM_TOKEN_ENV,
            before
                .and_then(|worker| worker.get("token"))
                .and_then(JsonValue::as_str),
            "Telegram bot token",
            "--telegram-token",
            resolved,
        )?);
        if resolved.telegram_username.is_none() {
            resolved.telegram_username = before
                .and_then(|worker| worker.get("username"))
                .and_then(JsonValue::as_str)
                .map(str::to_string)
                .or(optional_prompt(
                    None,
                    "Telegram bot username (optional)",
                    resolved,
                )?);
        }
    }
    if attach == "discord" || attach == "both" {
        let key = format!("discord/{}", resolved.worker_name);
        let before = before_workers.get(&key).and_then(JsonValue::as_object);
        resolved.discord_application_id = Some(resolve_secret(
            resolved.discord_application_id.as_deref(),
            DISCORD_APPLICATION_ID_ENV,
            before
                .and_then(|worker| worker.get("application_id"))
                .and_then(JsonValue::as_str),
            "Discord application id",
            "--discord-application-id",
            resolved,
        )?);
        resolved.discord_bot_token = Some(resolve_secret(
            resolved.discord_bot_token.as_deref(),
            DISCORD_BOT_TOKEN_ENV,
            before
                .and_then(|worker| worker.get("bot_token"))
                .and_then(JsonValue::as_str),
            "Discord bot token",
            "--discord-bot-token",
            resolved,
        )?);
    }
    Ok(())
}

fn resolve_secret(
    requested: Option<&str>,
    environment_key: &str,
    existing: Option<&str>,
    prompt: &str,
    option_name: &str,
    request: &InstallRequest,
) -> Result<String, String> {
    if let Some(value) = requested.and_then(|value| normalize_text(value.to_string())) {
        return Ok(value);
    }
    if let Some(value) = env::var(environment_key).ok().and_then(normalize_text) {
        return Ok(value);
    }
    if let Some(value) = existing.and_then(|value| normalize_text(value.to_string())) {
        return Ok(value);
    }
    if request.json_output || !request.interactive {
        return Err(format!(
            "Missing required value for {prompt}. Set `{environment_key}` or pass `{option_name}`."
        ));
    }
    rpassword::prompt_password(format!("{prompt}: "))
        .map_err(|err| format!("Failed to read {prompt}: {err}"))
        .and_then(|value| {
            normalize_text(value).ok_or_else(|| format!("Missing required value for {prompt}."))
        })
}

fn configure_transports(
    repo_root: &Path,
    attach: &str,
    before_workers: &JsonMap<String, JsonValue>,
    request: &InstallRequest,
) -> Result<JsonValue, String> {
    let mut actions = Vec::new();
    let manifest_path = agent_config_path(repo_root);
    let mut updated_config = if attach != "none" && !request.dry_run {
        Some(read_worker_manifest_config(&manifest_path)?)
    } else {
        None
    };
    if attach == "telegram" || attach == "both" {
        let token = request
            .telegram_token
            .clone()
            .ok_or_else(|| "Resolved Telegram bot token is missing.".to_string())?;
        let username = request.telegram_username.clone();
        let mut worker = JsonMap::new();
        worker.insert(
            "kind".to_string(),
            JsonValue::String("telegram".to_string()),
        );
        worker.insert(
            "name".to_string(),
            JsonValue::String(request.worker_name.clone()),
        );
        worker.insert("token".to_string(), JsonValue::String(token.clone()));
        if let Some(username) = username.as_ref() {
            worker.insert("username".to_string(), JsonValue::String(username.clone()));
        }
        let key = format!("telegram/{}", request.worker_name);
        let before = before_workers.get(&key).and_then(JsonValue::as_object);
        let action = worker_action(
            before,
            &[
                ("token", Some(token.as_str())),
                ("username", username.as_deref()),
            ],
        );
        if let Some(config) = updated_config.take() {
            updated_config = Some(upsert_agent_worker_config(
                &manifest_path,
                config,
                JsonValue::Object(worker),
            )?);
        }
        actions.push(json!({
            "kind": "telegram",
            "name": request.worker_name.as_str(),
            "action": if request.dry_run { preview_action(&action) } else { action },
            "configured": true,
        }));
    }
    if attach == "discord" || attach == "both" {
        let application_id = request
            .discord_application_id
            .clone()
            .ok_or_else(|| "Resolved Discord application id is missing.".to_string())?;
        let bot_token = request
            .discord_bot_token
            .clone()
            .ok_or_else(|| "Resolved Discord bot token is missing.".to_string())?;
        let mut worker = JsonMap::new();
        worker.insert("kind".to_string(), JsonValue::String("discord".to_string()));
        worker.insert(
            "name".to_string(),
            JsonValue::String(request.worker_name.clone()),
        );
        worker.insert(
            "application_id".to_string(),
            JsonValue::String(application_id.clone()),
        );
        worker.insert(
            "bot_token".to_string(),
            JsonValue::String(bot_token.clone()),
        );
        let key = format!("discord/{}", request.worker_name);
        let before = before_workers.get(&key).and_then(JsonValue::as_object);
        let action = worker_action(
            before,
            &[
                ("application_id", Some(application_id.as_str())),
                ("bot_token", Some(bot_token.as_str())),
            ],
        );
        if let Some(config) = updated_config.take() {
            updated_config = Some(upsert_agent_worker_config(
                &manifest_path,
                config,
                JsonValue::Object(worker),
            )?);
        }
        actions.push(json!({
            "kind": "discord",
            "name": request.worker_name.as_str(),
            "action": if request.dry_run { preview_action(&action) } else { action },
            "configured": true,
        }));
    }
    if let Some(config) = updated_config {
        write_worker_manifest(&manifest_path, &config)?;
    }
    Ok(JsonValue::Array(actions))
}

fn classify_runtime_root(repo_root: &Path) -> JsonValue {
    match doctor_runtime_root(repo_root, None) {
        Ok(report) => {
            let classification = if report
                .get("runtime_root_source")
                .and_then(JsonValue::as_str)
                == Some("unconfigured")
            {
                "installed_but_not_configured"
            } else if report.get("state").and_then(JsonValue::as_str) == Some("pass") {
                "healthy"
            } else {
                "configured_but_unhealthy"
            };
            json!({"classification": classification, "report": report})
        }
        Err(err) => json!({
            "classification": "configured_but_unhealthy",
            "report": {
                "state": "fail",
                "issues": [err],
            },
        }),
    }
}

fn starter_sprint_payload(
    repo_info: &JsonValue,
    mode: &str,
    sprint_enabled: bool,
    default_line: &str,
) -> JsonValue {
    let is_fresh = repo_info
        .get("state")
        .and_then(JsonValue::as_str)
        .is_some_and(|state| state == "initialized_repo" || state == "missing_repo");
    if !is_fresh || !sprint_enabled {
        return JsonValue::Null;
    }
    let local_flag = if is_remote_mode(mode) { "" } else { " --local" };
    json!({
        "path": "docs/sprints/first_change.md",
        "template": "# First change [plan-ref: first-change/root]\n\n## Intent\n\nDescribe the first bounded outcome.\n\n## Work Item\n\n- [ ] Implement and verify the first bounded change. [ref: first-change/implementation]\n\n## Validation\n\n- Run the narrowest relevant validation.\n",
        "task_start_command": format!(
            "ait task start{local_flag} --from docs/sprints/first_change.md#first-change/implementation --intent \"Implement and verify the first bounded change\" --base-line {default_line}"
        ),
    })
}

fn mode_next_steps(
    mode: &str,
    sprint_enabled: bool,
    fresh_install: bool,
    transport_actions: &[JsonValue],
    server_setup: &JsonMap<String, JsonValue>,
) -> Vec<String> {
    let mut steps = if !is_remote_mode(mode) {
        let mut local = Vec::new();
        if sprint_enabled {
            if fresh_install {
                local.push("Create `docs/sprints/first_change.md` from the exact `starter_sprint.template` included in this install result.".to_string());
                local.push("Run the exact command in `starter_sprint.task_start_command` after adapting the starter card to the first bounded outcome.".to_string());
            } else {
                local.push(
                    "Turn the requirement into the right Markdown sprint card first.".to_string(),
                );
                local.push("Start the task for that requirement with `ait task start --from <markdown-file>#<exact-ref> --intent \"<intent>\"`; this performs the exact-file Plan sync and derives the title.".to_string());
            }
        } else {
            local.push("Turn the requirement into the right Markdown artifact first.".to_string());
            local.push("Start the unbound task with `ait task start --title \"<title>\" --intent \"<intent>\"`; `--from` is unavailable while sprint mode is off.".to_string());
        }
        local.push("Carry the change through `ait task land <task-or-change-id> --local` when the local slice is ready.".to_string());
        local
    } else {
        let remote_name = object_string(Some(server_setup), "remote_name")
            .unwrap_or_else(|| DEFAULT_REMOTE_NAME.to_string());
        let server_configured = object_string(Some(server_setup), "classification").as_deref()
            == Some("configured_unverified");
        let mut remote = if fresh_install && sprint_enabled {
            vec!["Create `docs/sprints/first_change.md` from the exact `starter_sprint.template` included in this install result.".to_string()]
        } else {
            vec!["Turn the requirement into the right Markdown artifact first.".to_string()]
        };
        for step in server_setup
            .get("next_steps")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(JsonValue::as_str)
        {
            append_unique_step(&mut remote, step);
        }
        if server_configured {
            if sprint_enabled {
                append_unique_step(
                    &mut remote,
                    if fresh_install {
                        "After server verification succeeds, run the exact command in `starter_sprint.task_start_command` after adapting the starter card to the first bounded outcome."
                    } else {
                        "After server verification succeeds, start the remote-backed task with `ait task start --from <markdown-file>#<exact-ref> --intent \"<intent>\"`; this publishes the exact-file Plan lineage and derives the title."
                    },
                );
            } else {
                append_unique_step(
                    &mut remote,
                    "After server verification succeeds, start the remote-backed unbound task with `ait task start --title \"<title>\" --intent \"<intent>\"`; `--from` is unavailable while sprint mode is off.",
                );
            }
        } else {
            if sprint_enabled {
                let after_connect_step = if fresh_install {
                    format!(
                        "After `{remote_name}` is connected and verified, run the exact command in `starter_sprint.task_start_command` after adapting the starter card."
                    )
                } else {
                    format!(
                        "After `{remote_name}` is connected, start the remote-backed task with `ait task start --from <markdown-file>#<exact-ref> --intent \"<intent>\"`; this owns exact-file Plan publication and title derivation."
                    )
                };
                append_unique_step(&mut remote, &after_connect_step);
            } else {
                append_unique_step(
                    &mut remote,
                    "Then start the remote-backed unbound task with `ait task start --title \"<title>\" --intent \"<intent>\"`; `--from` remains unavailable while sprint mode is off.",
                );
            }
        }
        append_unique_step(
            &mut remote,
            "Carry the change through `ait task land <task-or-change-id>` once the shared slice is ready.",
        );
        remote
    };
    for item in transport_actions {
        if item
            .get("kind")
            .and_then(JsonValue::as_str)
            .is_some_and(|kind| kind == "telegram" || kind == "discord")
        {
            let kind = item
                .get("kind")
                .and_then(JsonValue::as_str)
                .unwrap_or("worker");
            let name = item
                .get("name")
                .and_then(JsonValue::as_str)
                .unwrap_or(DEFAULT_WORKER_NAME);
            steps.push(format!(
                "{} worker `{}` was configured but not started automatically; use `ait-agent {} start {}` when you are ready.",
                title_case(kind),
                name,
                kind,
                name
            ));
        }
    }
    if transport_actions.is_empty() {
        steps.push(
            "You can add Telegram or Discord later by rerunning `ait install --attach ...`."
                .to_string(),
        );
    }
    steps
}

fn append_unique_step(steps: &mut Vec<String>, step: &str) {
    if !step.is_empty() && !steps.iter().any(|existing| existing == step) {
        steps.push(step.to_string());
    }
}

fn load_agent_workers(repo_root: &Path) -> Result<JsonMap<String, JsonValue>, String> {
    let path = agent_config_path(repo_root);
    let config = read_worker_manifest_config(&path)?;
    Ok(match config.get("workers") {
        Some(JsonValue::Object(workers)) => workers.clone(),
        _ => JsonMap::new(),
    })
}

fn upsert_agent_worker_config(
    path: &Path,
    config: JsonValue,
    worker: JsonValue,
) -> Result<JsonValue, String> {
    let upserted = upsert_worker_manifest_worker_json(&json!({
        "path": path.to_string_lossy().to_string(),
        "config": config,
        "worker": worker,
        "updated_at": Utc::now().to_rfc3339(),
    }))?;
    upserted
        .get("config")
        .cloned()
        .ok_or_else(|| "worker manifest upsert did not return config.".to_string())
}

fn read_worker_manifest_config(path: &Path) -> Result<JsonValue, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "Refusing symbolic-link worker manifest: {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "Worker manifest must be a regular file: {}",
            path.display()
        )),
        Ok(_) => {
            let content = fs::read_to_string(path).map_err(|err| err.to_string())?;
            match parse_value(&content, &format!("Failed to parse {}", path.display()))? {
                value @ JsonValue::Object(_) => Ok(value),
                _ => Err(format!(
                    "Worker manifest must contain a JSON object: {}",
                    path.display()
                )),
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            Ok(default_worker_manifest_config_json())
        }
        Err(err) => Err(err.to_string()),
    }
}

fn write_worker_manifest(path: &Path, value: &JsonValue) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Worker manifest path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(|err| err.to_string())?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(format!(
            "Worker manifest parent must be a regular directory: {}",
            parent.display()
        ));
    }

    let encoded = encode_value_pretty_with_newline_error_string(value)?;
    let mut staged = NamedTempFile::new_in(parent).map_err(|err| err.to_string())?;
    #[cfg(unix)]
    staged
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|err| err.to_string())?;
    staged
        .as_file_mut()
        .write_all(encoded.as_bytes())
        .and_then(|_| staged.as_file_mut().flush())
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|err| err.to_string())?;
    staged.persist(path).map_err(|err| {
        format!(
            "Failed to atomically install worker manifest {}: {}",
            path.display(),
            err.error
        )
    })?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|err| err.to_string())?;
    Ok(())
}

fn secure_worker_manifest(
    repo_root: &Path,
    dry_run: bool,
    transport_planned: bool,
) -> Result<JsonValue, String> {
    let path = agent_config_path(repo_root);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "Refusing symbolic-link worker manifest: {}",
                path.display()
            ))
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!(
                "Worker manifest must be a regular file: {}",
                path.display()
            ))
        }
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(json!({
                "path": path.to_string_lossy().to_string(),
                "storage": "plaintext_json",
                "security_action": if dry_run && transport_planned { "would_create_owner_only" } else { "not_present" },
            }))
        }
        Err(err) => return Err(err.to_string()),
    };

    #[cfg(unix)]
    let security_action = {
        let mode = metadata.permissions().mode() & 0o777;
        if mode == 0o600 {
            "unchanged"
        } else if dry_run {
            "would_restrict_to_owner"
        } else {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(|err| err.to_string())?;
            "restricted_to_owner"
        }
    };
    #[cfg(not(unix))]
    let security_action = {
        let _ = metadata;
        "platform_acl_preserved"
    };

    Ok(json!({
        "path": path.to_string_lossy().to_string(),
        "storage": "plaintext_json",
        "security_action": security_action,
    }))
}

fn worker_action(
    before: Option<&JsonMap<String, JsonValue>>,
    desired_fields: &[(&str, Option<&str>)],
) -> String {
    let Some(before) = before else {
        return "created".to_string();
    };
    for (field, desired) in desired_fields {
        let before_text = before.get(*field).and_then(JsonValue::as_str).unwrap_or("");
        if before_text != desired.unwrap_or("") {
            return "updated".to_string();
        }
    }
    "unchanged".to_string()
}

fn preview_action(action: &str) -> String {
    match action {
        "created" => "would_create".to_string(),
        "updated" => "would_update".to_string(),
        other => format!("would_{other}"),
    }
}

fn existing_remote_matches(row: &JsonValue, url: &str, repo_name: Option<&str>) -> bool {
    let current_url = row
        .get("url")
        .and_then(JsonValue::as_str)
        .map(|value| value.trim_end_matches('/'))
        .unwrap_or("");
    if current_url != url {
        return false;
    }
    if let Some(repo_name) = repo_name {
        row.get("repo_name")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            == repo_name
    } else {
        true
    }
}

fn agent_config_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".ait").join("agent-workers.json")
}

fn require_text(
    value: Option<&str>,
    prompt: &str,
    request: &InstallRequest,
) -> Result<String, String> {
    if let Some(value) = value.and_then(|value| normalize_text(value.to_string())) {
        return Ok(value);
    }
    if request.json_output || !request.interactive {
        return Err(format!("Missing required value for {prompt}."));
    }
    prompt_line(prompt, None).and_then(|value| {
        normalize_text(value).ok_or_else(|| format!("Missing required value for {prompt}."))
    })
}

fn optional_prompt(
    value: Option<&str>,
    prompt: &str,
    request: &InstallRequest,
) -> Result<Option<String>, String> {
    if let Some(value) = value {
        return Ok(normalize_text(value.to_string()));
    }
    if request.json_output || !request.interactive {
        return Ok(None);
    }
    prompt_line(prompt, Some("")).map(normalize_text)
}

fn prompt_choice(prompt: &str, default: &str, choices: &[&str]) -> Result<String, String> {
    let raw = prompt_line(prompt, Some(default))?;
    let value = raw.trim().to_lowercase();
    if choices.iter().any(|choice| *choice == value) {
        return Ok(value);
    }
    Err(format!("{prompt} must be one of: {}.", choices.join(", ")))
}

fn prompt_confirm(prompt: &str, default: bool) -> Result<bool, String> {
    let default_text = if default { "Y/n" } else { "y/N" };
    let raw = prompt_line(&format!("{prompt} [{default_text}]"), Some(""))?;
    let value = raw.trim().to_lowercase();
    if value.is_empty() {
        return Ok(default);
    }
    match value.as_str() {
        "y" | "yes" | "true" | "on" | "1" => Ok(true),
        "n" | "no" | "false" | "off" | "0" => Ok(false),
        _ => Err(format!("{prompt} must be yes or no.")),
    }
}

fn prompt_line(prompt: &str, default: Option<&str>) -> Result<String, String> {
    match default {
        Some(default) if !default.is_empty() => print!("{prompt} [{default}]: "),
        _ => print!("{prompt}: "),
    }
    io::stdout().flush().map_err(|err| err.to_string())?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|err| err.to_string())?;
    let text = line.trim().to_string();
    if text.is_empty() {
        Ok(default.unwrap_or("").to_string())
    } else {
        Ok(text)
    }
}

fn replace_object_field(
    payload: &mut JsonValue,
    key: &str,
    value: JsonValue,
) -> Result<(), String> {
    payload
        .as_object_mut()
        .ok_or_else(|| "install payload must be an object.".to_string())?
        .insert(key.to_string(), value);
    Ok(())
}

fn normalize_text(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn optional_string_field(
    object: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => Ok(Some(value.clone())),
        Some(JsonValue::Number(number)) => Ok(Some(number.to_string())),
        Some(JsonValue::Bool(flag)) => Ok(Some(flag.to_string())),
        Some(_) => Err(format!(
            "install payload field `{key}` must be scalar text."
        )),
    }
}

fn optional_bool_field(
    object: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<bool>, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(value)) => Ok(Some(*value)),
        Some(JsonValue::String(value)) => match value.trim().to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(Some(true)),
            "false" | "0" | "no" | "off" => Ok(Some(false)),
            _ => Err(format!("install payload field `{key}` must be boolean.")),
        },
        Some(_) => Err(format!("install payload field `{key}` must be boolean.")),
    }
}

fn bool_field(object: &JsonMap<String, JsonValue>, key: &str) -> Result<bool, String> {
    Ok(optional_bool_field(object, key)?.unwrap_or(false))
}

fn object_string(object: Option<&JsonMap<String, JsonValue>>, key: &str) -> Option<String> {
    object
        .and_then(|value| value.get(key))
        .and_then(|value| match value {
            JsonValue::String(text) => Some(text.clone()),
            JsonValue::Bool(flag) => Some(flag.to_string()),
            JsonValue::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .and_then(normalize_text)
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn temp_dir_outside_repo() -> TempDir {
        let system_tmp = Path::new("/tmp");
        if system_tmp.is_dir() {
            tempfile::Builder::new()
                .prefix("ait-install-test-")
                .tempdir_in(system_tmp)
                .unwrap()
        } else {
            TempDir::new().unwrap()
        }
    }

    #[test]
    fn install_defaults_sprint_on_and_required_binding() {
        let temp = temp_dir_outside_repo();
        let payload = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "user_name": "Ada Lovelace",
            "user_email": "ada@example.test",
            "json_output": true,
        }))
        .expect("install");

        assert_eq!(payload["repository"]["state"], "initialized_repo");
        assert_eq!(payload["sprint"]["value"], "on");
        assert_eq!(payload["sprint"]["plan_task_binding_mode"], "required");
        assert_eq!(payload["identity"]["user_name"], "Ada Lovelace");
        assert_eq!(payload["identity"]["user_email"], "ada@example.test");
        assert_eq!(payload["identity"]["action"], "updated");
        assert_eq!(payload["agent_harness"]["status"], "synced");
        assert_eq!(payload["agent_harness"]["scope"], "local");
        assert_eq!(payload["agent_harness"]["plan_sync"]["status"], "ok");
        let next_steps = payload["next_steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(JsonValue::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(next_steps.contains("starter_sprint.task_start_command"));
        assert!(next_steps.contains("ait task land <task-or-change-id> --local"));
        assert!(!next_steps.contains("workflow land-local"));
        assert!(!next_steps.contains("--plan-item-ref"));
        assert_eq!(
            payload["starter_sprint"]["path"],
            "docs/sprints/first_change.md"
        );
        assert!(payload["starter_sprint"]["template"]
            .as_str()
            .unwrap()
            .contains("[plan-ref: first-change/root]"));
        assert!(payload["starter_sprint"]["task_start_command"]
            .as_str()
            .unwrap()
            .contains("docs/sprints/first_change.md#first-change/implementation"));
        let rendered = render_install_text(&payload);
        assert!(rendered.contains("Starter sprint card: docs/sprints/first_change.md"));
        assert!(rendered.contains("[ref: first-change/implementation]"));
        assert!(rendered.contains("Then run: ait task start --local"));

        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        assert_eq!(repo.config["workflow_mode"], "solo_local");
        assert_eq!(repo.config["sprint"], "on");
        assert_eq!(repo.config["plan_task_binding"]["mode"], "required");
        assert_eq!(repo.config["user_name"], "Ada Lovelace");
        assert_eq!(repo.config["user_email"], "ada@example.test");
        let agents = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("`ait init`, `ait install`, relevant `ait config set`"));
        assert!(agents.contains("ait blame <path>"));
        assert!(
            agents.contains("After every context-window compaction, re-read the bound sprint card")
        );
        assert!(agents.split_whitespace().count() < 1_024);
    }

    #[test]
    fn install_can_disable_sprint_binding() {
        let temp = temp_dir_outside_repo();
        let payload = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "sprint": false,
            "json_output": true,
        }))
        .expect("install");

        assert_eq!(payload["sprint"]["value"], "off");
        assert_eq!(payload["sprint"]["plan_task_binding_mode"], "off");

        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        assert_eq!(repo.config["sprint"], "off");
        assert_eq!(repo.config["plan_task_binding"]["mode"], "off");
        assert_eq!(repo.effective_workflow_mode(), "solo_local");
        let next_steps = payload["next_steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(JsonValue::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(next_steps.contains("task start --title \"<title>\""));
        assert!(next_steps.contains("`--from` is unavailable"));
        assert!(!next_steps.contains("--plan-item-ref"));
    }

    #[test]
    fn install_no_init_fails_before_requesting_transport_secrets() {
        let temp = temp_dir_outside_repo();
        let error = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "initialize": false,
            "attach": "telegram",
            "json_output": true,
        }))
        .expect_err("missing repository must fail first");

        assert!(error.contains("No `ait` repository found"));
        assert!(!error.contains(TELEGRAM_TOKEN_ENV));
    }

    #[test]
    fn install_noop_repairs_and_syncs_the_generated_agents_block() {
        let temp = temp_dir_outside_repo();
        install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "json_output": true,
        }))
        .expect("initial install");
        fs::write(
            temp.path().join("AGENTS.md"),
            "# AGENTS\n\nCustom repository guidance.\n",
        )
        .unwrap();

        let payload = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "json_output": true,
        }))
        .expect("idempotent install");

        assert_eq!(payload["mode"]["action"], "unchanged");
        assert_eq!(payload["agent_harness"]["status"], "synced");
        assert_eq!(payload["agent_harness"]["refresh"]["status"], "updated");
        let agents = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("Custom repository guidance."));
        assert!(agents.contains("<!-- ait:workflow:start -->"));
        assert!(agents.contains("workflow mode: `solo_local`"));
    }

    #[test]
    fn install_rerun_preserves_existing_remote_mode_and_disabled_sprint() {
        let temp = temp_dir_outside_repo();
        install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "mode": "remote",
            "server_setup": "skip",
            "sprint": false,
            "json_output": true,
        }))
        .expect("initial remote install");

        let payload = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "json_output": true,
        }))
        .expect("preserving rerun");

        assert_eq!(payload["mode"]["requested_mode"], "solo_remote");
        assert_eq!(payload["mode"]["effective_mode"], "solo_remote");
        assert_eq!(payload["mode"]["source"], "existing_repository");
        assert_eq!(payload["mode"]["action"], "unchanged");
        assert_eq!(payload["sprint"]["value"], "off");
        assert_eq!(payload["sprint"]["source"], "existing_repository");

        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        assert_eq!(repo.effective_workflow_mode(), "solo_remote");
        assert!(!repo.sprint_enabled());

        let changed_mode = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "mode": "local",
            "json_output": true,
        }))
        .expect("explicit mode-only change");
        assert_eq!(changed_mode["mode"]["effective_mode"], "solo_local");
        assert_eq!(changed_mode["sprint"]["value"], "off");
        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        assert_eq!(repo.effective_workflow_mode(), "solo_local");
        assert!(!repo.sprint_enabled());
    }

    #[test]
    fn install_remote_connect_dry_run_is_configured_but_unverified() {
        let temp = temp_dir_outside_repo();
        install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "json_output": true,
        }))
        .expect("initial install");

        let payload = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "mode": "remote",
            "server_setup": "connect",
            "server_url": "http://127.0.0.1:1",
            "remote_name": "unreachable",
            "dry_run": true,
            "json_output": true,
        }))
        .expect("remote dry run");

        assert_eq!(payload["server"]["action"], "would_create");
        assert_eq!(payload["server"]["classification"], "configured_unverified");
        assert!(payload["server"]["next_steps"][0]
            .as_str()
            .unwrap()
            .contains("Verify the configured ait-server"));
    }

    #[test]
    fn install_remote_deploy_guidance_uses_postgresql_free_binary_server_lifecycle() {
        let temp = temp_dir_outside_repo();
        let payload = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "mode": "remote",
            "server_setup": "deploy",
            "json_output": true,
        }))
        .expect("remote deploy guidance");

        assert_eq!(payload["server"]["action"], "guidance_only");
        assert!(payload.get("postgres").is_none());
        let steps = payload["server"]["next_steps"]
            .as_array()
            .expect("server next steps")
            .iter()
            .filter_map(JsonValue::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(steps.contains("ait-server init"));
        assert!(steps.contains("ait-server probe --defer-ci-admission"));
        assert!(steps.contains("ait-server run --init-if-missing --defer-ci-admission"));
        assert!(!steps.contains("doctor postgres"));
        assert!(!steps.contains("configure PostgreSQL"));
    }

    #[test]
    fn install_attach_uses_rust_worker_manifest_upsert() {
        let temp = temp_dir_outside_repo();
        let payload = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "attach": "telegram",
            "telegram_token": "secret",
            "telegram_username": "bot",
            "json_output": true,
        }))
        .expect("install");

        assert_eq!(payload["transport_actions"][0]["action"], "created");
        let manifest = parse_value(
            &fs::read_to_string(temp.path().join(".ait/agent-workers.json")).unwrap(),
            "manifest",
        )
        .unwrap();
        assert_eq!(manifest["workers"]["telegram/main"]["token"], "secret");
        assert_eq!(manifest["workers"]["telegram/main"]["username"], "bot");
        assert_eq!(payload["worker_manifest"]["storage"], "plaintext_json");
        #[cfg(unix)]
        {
            let mode = fs::metadata(temp.path().join(".ait/agent-workers.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn install_attach_both_stages_one_complete_worker_manifest() {
        let temp = temp_dir_outside_repo();
        let payload = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "attach": "both",
            "telegram_token": "telegram-secret",
            "telegram_username": "telegram-bot",
            "discord_application_id": "discord-app",
            "discord_bot_token": "discord-secret",
            "json_output": true,
        }))
        .expect("install both workers");

        assert_eq!(payload["transport_actions"][0]["action"], "created");
        assert_eq!(payload["transport_actions"][1]["action"], "created");
        let manifest = parse_value(
            &fs::read_to_string(temp.path().join(".ait/agent-workers.json")).unwrap(),
            "manifest",
        )
        .unwrap();
        assert_eq!(
            manifest["workers"]["telegram/main"]["token"],
            "telegram-secret"
        );
        assert_eq!(
            manifest["workers"]["discord/main"]["application_id"],
            "discord-app"
        );
        assert_eq!(
            manifest["workers"]["discord/main"]["bot_token"],
            "discord-secret"
        );
    }

    #[cfg(unix)]
    #[test]
    fn install_rerun_repairs_existing_worker_manifest_permissions() {
        let temp = temp_dir_outside_repo();
        install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "attach": "telegram",
            "telegram_token": "secret",
            "json_output": true,
        }))
        .expect("install worker");
        let path = temp.path().join(".ait/agent-workers.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let payload = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "json_output": true,
        }))
        .expect("repair permissions");

        assert_eq!(
            payload["worker_manifest"]["security_action"],
            "restricted_to_owner"
        );
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn install_rejects_symbolic_link_worker_manifest() {
        let temp = temp_dir_outside_repo();
        install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "json_output": true,
        }))
        .expect("initial install");
        let target = temp.path().join("outside-worker-manifest.json");
        fs::write(&target, "{}\n").unwrap();
        std::os::unix::fs::symlink(&target, temp.path().join(".ait/agent-workers.json")).unwrap();

        let error = install_from_payload(&json!({
            "cwd": temp.path().to_string_lossy().to_string(),
            "json_output": true,
        }))
        .expect_err("symbolic link must fail closed");

        assert!(error.contains("Refusing symbolic-link worker manifest"));
        assert_eq!(fs::read_to_string(target).unwrap(), "{}\n");
    }
}
