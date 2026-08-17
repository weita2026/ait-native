use crate::agent_harness::converge_agent_workflow_harness;
use crate::json_support::{encode_value_pretty_with_newline_error_string, parse_object_or_empty};
use crate::runtime::RepoRuntime;
use crate::task_worktree_layout::config_task_worktree_summary;
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::server_operational::ServerRepositoryAuthorityConfig;
use std::env;
use std::fs;
use std::path::Path;

const DEFAULT_AUTHOR_MODE: &str = "ai_with_human_review";
const DEFAULT_WORKFLOW_SCOPE: &str = "local";
const DEFAULT_PLAN_TASK_BINDING_MODE: &str = "required";
const DEFAULT_ID_NAMESPACE_PREFIX: &str = "";
const WORKFLOW_ID_FAMILIES: &[&str] = &[
    "T", "C", "P", "R", "S", "PS", "K", "PL", "PR", "SK", "HP", "AM", "AN", "AMU", "STH",
];
const RESERVED_WORKFLOW_TOKENS: &[&str] = &["AT", "LAND", "W"];
#[derive(Clone, Debug, Default)]
pub struct ConfigSetRequest {
    pub default_author_mode: Option<String>,
    pub default_model: Option<String>,
    pub task_review: Option<String>,
    pub task_worktree_alias_root: Option<String>,
    pub task_worktree_main_seed_ram_max_bytes: Option<i64>,
    pub workflow_mode: Option<String>,
    pub id_namespace_prefix: Option<String>,
    pub sprint: Option<String>,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
}

impl ConfigSetRequest {
    pub fn updated_keys(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        if self.workflow_mode.is_some() {
            keys.push("workflow-mode");
            if self.sprint.is_none() {
                keys.push("sprint");
            }
        }
        if self.sprint.is_some() {
            keys.push("sprint");
        }
        if self.default_author_mode.is_some() {
            keys.push("default-author-mode");
        }
        if self.default_model.is_some() {
            keys.push("default-model");
        }
        if self.task_review.is_some() {
            keys.push("task-review");
        }
        if self.task_worktree_alias_root.is_some() {
            keys.push("task-worktree-alias-root");
        }
        if self.task_worktree_main_seed_ram_max_bytes.is_some() {
            keys.push("task-worktree-main-seed-ram-max-bytes");
        }
        if self.id_namespace_prefix.is_some() {
            keys.push("id-namespace-prefix");
        }
        if self.user_name.is_some() {
            keys.push("user-name");
        }
        if self.user_email.is_some() {
            keys.push("user-email");
        }
        keys
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigUnsetKey {
    DefaultAuthorMode,
    DefaultModel,
    TaskReview,
    TaskWorktreeAliasRoot,
    TaskWorktreeMainSeedRamMaxBytes,
    IdNamespacePrefix,
    UserName,
    UserEmail,
}

impl ConfigUnsetKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DefaultAuthorMode => "default-author-mode",
            Self::DefaultModel => "default-model",
            Self::TaskReview => "task-review",
            Self::TaskWorktreeAliasRoot => "task-worktree-alias-root",
            Self::TaskWorktreeMainSeedRamMaxBytes => "task-worktree-main-seed-ram-max-bytes",
            Self::IdNamespacePrefix => "id-namespace-prefix",
            Self::UserName => "user-name",
            Self::UserEmail => "user-email",
        }
    }
}

pub fn config_show(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let repository_index = ServerRepositoryAuthorityConfig::from_config_object(&repo.config)?
        .map(|config| config.repository_index.get());
    let workflow_default_scope = workflow_default_scope_summary(repo);
    let workflow_mode = workflow_mode_summary(repo, &workflow_default_scope);
    let worktree_name = repo
        .is_worktree()
        .then(|| config_text(repo, "worktree_name"))
        .flatten();
    let root_config = read_json_object(
        &repo
            .authoritative_repo_root()
            .join(".ait")
            .join("config.json"),
    );
    let active_root_worktree_name = root_config
        .get("worktree_name")
        .and_then(config_scalar_text);
    Ok(json!({
        "repo_root": repo.authoritative_repo_root().to_string_lossy().to_string(),
        "workspace_root": repo.workspace_root().to_string_lossy().to_string(),
        "is_worktree": repo.is_worktree(),
        "worktree_name": worktree_name,
        "active_root_worktree_name": active_root_worktree_name,
        "repo_name": repo.repo_name(),
        "repository_index": repository_index,
        "default_line": repo.default_line_name(),
        "current_line": repo.current_line_name()?,
        "default_remote": config_text(repo, "default_remote"),
        "policy_profile": config_text(repo, "policy_profile"),
        "default_author_mode": config_text(repo, "default_author_mode"),
        "default_model": config_text(repo, "default_model"),
        "detected_actor": detected_actor(),
        "user_name": config_text(repo, "user_name"),
        "user_email": config_text(repo, "user_email"),
        "effective_actor": effective_actor(repo),
        "effective_reviewer": effective_reviewer(repo),
        "effective_author_mode": effective_author_mode(repo),
        "effective_model": effective_model(repo),
        "task_review": task_review_summary(repo),
        "id_namespace_prefix": id_namespace_prefix_summary(repo),
        "workflow_mode": workflow_mode,
        "workflow_default_scope": workflow_default_scope,
        "agent_runtime": agent_runtime_summary(repo),
        "task_worktree": config_task_worktree_summary(repo),
        "sprint": sprint_summary(repo),
        "plan_task_binding": plan_task_binding_summary(repo),
        "web_inbox_defaults": web_inbox_defaults_summary(repo),
    }))
}

pub fn config_set(repo: &RepoRuntime, request: &ConfigSetRequest) -> Result<JsonValue, String> {
    validate_config_set_request(request)?;
    update_root_config(repo, |config| apply_config_set_updates(config, request))?;
    complete_config_mutation(repo)
}

pub fn config_unset(repo: &RepoRuntime, key: ConfigUnsetKey) -> Result<JsonValue, String> {
    update_root_config(repo, |config| apply_config_unset(config, key))?;
    complete_config_mutation(repo)
}

fn complete_config_mutation(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let refreshed = RepoRuntime::discover_from_path(&repo.root)?;
    let agent_harness = converge_agent_workflow_harness(&refreshed)?;
    let mut payload = config_show(&refreshed)?;
    payload
        .as_object_mut()
        .ok_or_else(|| "Config show payload must be an object.".to_string())?
        .insert("agent_harness".to_string(), agent_harness);
    Ok(payload)
}

pub fn config_set_from_payload(
    repo: &RepoRuntime,
    payload: &JsonValue,
) -> Result<JsonValue, String> {
    let request = parse_config_set_request(payload)?;
    config_set(repo, &request)
}

fn parse_config_set_request(payload: &JsonValue) -> Result<ConfigSetRequest, String> {
    let object = payload
        .as_object()
        .ok_or_else(|| "config set payload must decode to an object.".to_string())?;
    reject_unknown_config_set_fields(
        object,
        &[
            "default_author_mode",
            "default_model",
            "task_review",
            "task_worktree_alias_root",
            "task_worktree_main_seed_ram_max_bytes",
            "workflow_mode",
            "id_namespace_prefix",
            "sprint",
            "user_name",
            "user_email",
        ],
    )?;
    Ok(ConfigSetRequest {
        default_author_mode: option_string_field(object, "default_author_mode")?,
        default_model: option_string_field(object, "default_model")?,
        task_review: option_string_field(object, "task_review")?,
        task_worktree_alias_root: option_string_field(object, "task_worktree_alias_root")?,
        task_worktree_main_seed_ram_max_bytes: option_i64_field(
            object,
            "task_worktree_main_seed_ram_max_bytes",
        )?,
        workflow_mode: option_string_field(object, "workflow_mode")?,
        id_namespace_prefix: option_string_field(object, "id_namespace_prefix")?,
        sprint: option_string_field(object, "sprint")?,
        user_name: option_string_field(object, "user_name")?,
        user_email: option_string_field(object, "user_email")?,
    })
}

fn validate_config_set_request(request: &ConfigSetRequest) -> Result<(), String> {
    if !request_has_updates(request) {
        return Err("No config updates specified".to_string());
    }
    if let Some(value) = request.default_author_mode.as_deref() {
        normalize_author_mode_value(value)?;
    }
    for (value, option_name) in [
        (request.default_model.as_deref(), "--default-model"),
        (
            request.task_worktree_alias_root.as_deref(),
            "--task-worktree-alias-root",
        ),
        (request.user_name.as_deref(), "--user-name"),
        (request.user_email.as_deref(), "--user-email"),
    ] {
        if let Some(value) = value {
            require_nonempty_config_text(value, option_name)?;
        }
    }
    if let Some(value) = request.task_review.as_deref() {
        normalize_task_review_mode(value)?;
    }
    if request
        .task_worktree_main_seed_ram_max_bytes
        .is_some_and(|value| value < 0)
    {
        return Err(
            "`--task-worktree-main-seed-ram-max-bytes` must be a non-negative integer.".to_string(),
        );
    }
    if let Some(value) = request.workflow_mode.as_deref() {
        normalize_public_workflow_mode_value(value)?;
    }
    if let Some(value) = request.id_namespace_prefix.as_deref() {
        require_nonempty_config_text(value, "--id-namespace-prefix")?;
        normalize_id_namespace_prefix_value(value)?;
    }
    if let Some(value) = request.sprint.as_deref() {
        normalize_public_toggle_value(value, "--sprint")?;
    }
    Ok(())
}

fn request_has_updates(request: &ConfigSetRequest) -> bool {
    request.default_author_mode.is_some()
        || request.default_model.is_some()
        || request.task_review.is_some()
        || request.task_worktree_alias_root.is_some()
        || request.task_worktree_main_seed_ram_max_bytes.is_some()
        || request.workflow_mode.is_some()
        || request.id_namespace_prefix.is_some()
        || request.sprint.is_some()
        || request.user_name.is_some()
        || request.user_email.is_some()
}

fn require_nonempty_config_text(value: &str, option_name: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!(
            "`{option_name}` requires a non-empty value; use `ait config unset <KEY>` to remove an override."
        ));
    }
    Ok(())
}

fn reject_unknown_config_set_fields(
    object: &JsonMap<String, JsonValue>,
    allowed: &[&str],
) -> Result<(), String> {
    let mut unsupported = object
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    unsupported.sort();
    if unsupported.is_empty() {
        return Ok(());
    }
    Err(format!(
        "config set payload contains retired or unknown field(s): {}.",
        unsupported.join(", ")
    ))
}

fn apply_config_set_updates(
    config: &mut JsonMap<String, JsonValue>,
    request: &ConfigSetRequest,
) -> Result<(), String> {
    if let Some(value) = request.default_author_mode.as_ref() {
        config.insert(
            "default_author_mode".to_string(),
            JsonValue::String(normalize_author_mode_value(value)?),
        );
    }

    if let Some(value) = request.default_model.as_ref() {
        config.insert(
            "default_model".to_string(),
            JsonValue::String(value.trim().to_string()),
        );
    }

    if let Some(value) = request.task_review.as_ref() {
        config.insert(
            "task_review".to_string(),
            JsonValue::Bool(normalize_task_review_mode(value)?),
        );
    }

    if request.task_worktree_alias_root.is_some()
        || request.task_worktree_main_seed_ram_max_bytes.is_some()
    {
        let mut task_worktree_config = task_worktree_config_for_mutation(config)?;
        if let Some(value) = request.task_worktree_alias_root.as_ref() {
            task_worktree_config.insert(
                "alias_root".to_string(),
                JsonValue::String(value.trim().to_string()),
            );
        }
        if let Some(value) = request.task_worktree_main_seed_ram_max_bytes {
            task_worktree_config.insert(
                "main_seed_ram_max_bytes".to_string(),
                JsonValue::Number(value.into()),
            );
        }
        config.insert(
            "task_worktree".to_string(),
            JsonValue::Object(task_worktree_config),
        );
    }

    if let Some(value) = request.workflow_mode.as_ref() {
        let normalized = normalize_public_workflow_mode_value(value)?;
        let sprint = request
            .sprint
            .as_ref()
            .map(|value| normalize_public_toggle_value(value, "--sprint"))
            .transpose()?
            .unwrap_or_else(|| "on".to_string());
        config.insert(
            "workflow_mode".to_string(),
            JsonValue::String(normalized.clone()),
        );
        let preset = workflow_mode_preset(&normalized).unwrap();
        config.insert(
            "workflow_default_scope".to_string(),
            JsonValue::String(preset.workflow_scope.to_string()),
        );
        config.insert(
            "task_default_scope".to_string(),
            JsonValue::String(preset.task_scope.to_string()),
        );
        config.insert(
            "change_default_scope".to_string(),
            JsonValue::String(preset.change_scope.to_string()),
        );
        config.insert("sprint".to_string(), JsonValue::String(sprint.clone()));
        config.insert(
            "plan_task_binding".to_string(),
            json!({"mode": plan_task_binding_mode_for_sprint(&sprint)}),
        );
    } else if let Some(value) = request.sprint.as_ref() {
        let sprint = normalize_public_toggle_value(value, "--sprint")?;
        config.insert("sprint".to_string(), JsonValue::String(sprint.clone()));
        config.insert(
            "plan_task_binding".to_string(),
            json!({"mode": plan_task_binding_mode_for_sprint(&sprint)}),
        );
    }

    if let Some(value) = request.id_namespace_prefix.as_ref() {
        config.insert(
            "id_namespace_prefix".to_string(),
            JsonValue::String(normalize_id_namespace_prefix_value(value)?),
        );
    }

    if let Some(value) = request.user_name.as_ref() {
        config.insert(
            "user_name".to_string(),
            JsonValue::String(value.trim().to_string()),
        );
    }

    if let Some(value) = request.user_email.as_ref() {
        config.insert(
            "user_email".to_string(),
            JsonValue::String(value.trim().to_string()),
        );
    }
    Ok(())
}

fn apply_config_unset(
    config: &mut JsonMap<String, JsonValue>,
    key: ConfigUnsetKey,
) -> Result<(), String> {
    match key {
        ConfigUnsetKey::DefaultAuthorMode => {
            config.remove("default_author_mode");
        }
        ConfigUnsetKey::DefaultModel => {
            config.remove("default_model");
        }
        ConfigUnsetKey::TaskReview => {
            config.remove("task_review");
        }
        ConfigUnsetKey::TaskWorktreeAliasRoot | ConfigUnsetKey::TaskWorktreeMainSeedRamMaxBytes => {
            let Some(value) = config.get("task_worktree") else {
                return Ok(());
            };
            let mut task_worktree = value.as_object().cloned().ok_or_else(|| {
                "Cannot unset a task-worktree override because config.task_worktree is not an object."
                    .to_string()
            })?;
            let field = match key {
                ConfigUnsetKey::TaskWorktreeAliasRoot => "alias_root",
                ConfigUnsetKey::TaskWorktreeMainSeedRamMaxBytes => "main_seed_ram_max_bytes",
                _ => unreachable!(),
            };
            task_worktree.remove(field);
            if task_worktree.is_empty() {
                config.remove("task_worktree");
            } else {
                config.insert(
                    "task_worktree".to_string(),
                    JsonValue::Object(task_worktree),
                );
            }
        }
        ConfigUnsetKey::IdNamespacePrefix => {
            config.remove("id_namespace_prefix");
        }
        ConfigUnsetKey::UserName => {
            config.remove("user_name");
        }
        ConfigUnsetKey::UserEmail => {
            config.remove("user_email");
        }
    }
    Ok(())
}

fn task_worktree_config_for_mutation(
    config: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    match config.get("task_worktree") {
        None => Ok(JsonMap::new()),
        Some(JsonValue::Object(value)) => Ok(value.clone()),
        Some(_) => Err(
            "Cannot update a task-worktree override because config.task_worktree is not an object."
                .to_string(),
        ),
    }
}

fn config_text(repo: &RepoRuntime, key: &str) -> Option<String> {
    repo.config.get(key).and_then(json_text)
}

fn json_text(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => normalize_text(Some(text)),
        JsonValue::Number(number) => normalize_text(Some(&number.to_string())),
        JsonValue::Bool(flag) => normalize_text(Some(if *flag { "true" } else { "false" })),
        _ => None,
    }
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_author_mode_value(value: &str) -> Result<String, String> {
    match value {
        "human_only" | "human_with_ai_assist" | "ai_with_human_review"
        | "ai_only_experimental" => Ok(value.to_string()),
        _ => Err(
            "`--default-author-mode` must be exactly one of: human_only, human_with_ai_assist, ai_with_human_review, ai_only_experimental."
                .to_string(),
        ),
    }
}

fn normalize_toggle_mode(value: &str, option_name: &str) -> Result<String, String> {
    match normalize_text(Some(value))
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "on" | "true" | "yes" => Ok("on".to_string()),
        "off" | "false" | "no" => Ok("off".to_string()),
        _ => Err(format!("{option_name} must be `on` or `off`.")),
    }
}

fn normalize_public_toggle_value(value: &str, option_name: &str) -> Result<String, String> {
    match value {
        "on" | "off" => Ok(value.to_string()),
        _ => Err(format!("`{option_name}` must be exactly `on` or `off`.")),
    }
}

fn normalize_task_review_mode(value: &str) -> Result<bool, String> {
    match value {
        "required" => Ok(true),
        "automatic" => Ok(false),
        _ => Err("`--task-review` must be exactly `required` or `automatic`.".to_string()),
    }
}

fn normalize_stored_plan_task_binding_mode(value: &str) -> Result<String, String> {
    match normalize_text(Some(value))
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "off" | "advisory" | "strict" | "required" => Ok(value.trim().to_lowercase()),
        _ => Err(
            "Stored plan_task_binding.mode must be off, advisory, strict, or required.".to_string(),
        ),
    }
}

fn plan_task_binding_mode_for_sprint(sprint: &str) -> &'static str {
    if sprint == "on" {
        "required"
    } else {
        "off"
    }
}

fn normalize_workflow_scope_value(value: &str, option_name: &str) -> Result<String, String> {
    match normalize_text(Some(value))
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "local" | "remote" => Ok(value.trim().to_lowercase()),
        _ => Err(format!("{option_name} must be `local` or `remote`.")),
    }
}

fn normalize_workflow_mode_value(value: &str, option_name: &str) -> Result<String, String> {
    match normalize_text(Some(value))
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "solo_local" | "solo_remote" | "team_remote" => Ok(value.trim().to_lowercase()),
        _ => Err(format!(
            "{option_name} must be `solo_local`, `solo_remote`, or `team_remote`."
        )),
    }
}

fn normalize_public_workflow_mode_value(value: &str) -> Result<String, String> {
    match value {
        "solo_local" | "solo_remote" | "team_remote" => Ok(value.to_string()),
        _ => Err(
            "`--workflow-mode` must be exactly `solo_local`, `solo_remote`, or `team_remote`."
                .to_string(),
        ),
    }
}

fn normalize_id_namespace_prefix_value(value: &str) -> Result<String, String> {
    let text = value.trim().to_uppercase();
    if !text.is_empty() && !text.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err("id namespace prefix must contain only ASCII letters or digits".to_string());
    }
    for family in WORKFLOW_ID_FAMILIES {
        let token = if text.is_empty() {
            (*family).to_string()
        } else {
            format!("{text}{family}")
        };
        if RESERVED_WORKFLOW_TOKENS.contains(&token.as_str()) {
            return Err(format!(
                "id namespace prefix {text:?} collides with reserved workflow token {token:?}"
            ));
        }
    }
    Ok(text)
}

fn detected_actor() -> Option<String> {
    env::var(ait_core::environment_contract::names::AIT_NATIVE_ACTOR)
        .ok()
        .and_then(|value| normalize_text(Some(&value)))
}

fn effective_actor(repo: &RepoRuntime) -> Option<String> {
    detected_actor()
        .or_else(|| config_text(repo, "user_email"))
        .or_else(|| config_text(repo, "user_name"))
}

fn effective_reviewer(repo: &RepoRuntime) -> Option<String> {
    let user_name = config_text(repo, "user_name");
    let user_email = config_text(repo, "user_email");
    match (user_name, user_email) {
        (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
        (None, Some(email)) => Some(email),
        (Some(name), None) => Some(name),
        (None, None) => detected_actor(),
    }
}

fn effective_author_mode(repo: &RepoRuntime) -> String {
    config_text(repo, "default_author_mode").unwrap_or_else(|| DEFAULT_AUTHOR_MODE.to_string())
}

fn effective_model(repo: &RepoRuntime) -> Option<String> {
    config_text(repo, "default_model")
}

fn task_review_summary(repo: &RepoRuntime) -> JsonValue {
    let (value, source) = match configured_task_review(repo) {
        Some(true) => ("required", "repo_config"),
        Some(false) => ("automatic", "repo_config"),
        None => ("automatic", "built_in"),
    };
    json!({
        "value": value,
        "source": source,
        "automatic_reviewer": if value == "automatic" {
            config_text(repo, "user_name")
        } else {
            None
        },
    })
}

fn configured_task_review(repo: &RepoRuntime) -> Option<bool> {
    repo.config.get("task_review").and_then(JsonValue::as_bool)
}

fn configured_id_namespace_prefix(repo: &RepoRuntime) -> Option<String> {
    if !repo.config.contains_key("id_namespace_prefix") {
        return None;
    }
    match repo.config.get("id_namespace_prefix") {
        Some(JsonValue::String(text)) => normalize_id_namespace_prefix_value(text)
            .ok()
            .or_else(|| Some(DEFAULT_ID_NAMESPACE_PREFIX.to_string())),
        Some(value) => json_text(value)
            .map(|value| normalize_id_namespace_prefix_value(&value))
            .transpose()
            .ok()
            .flatten()
            .or_else(|| Some(DEFAULT_ID_NAMESPACE_PREFIX.to_string())),
        None => None,
    }
}

fn id_namespace_prefix_summary(repo: &RepoRuntime) -> JsonValue {
    match configured_id_namespace_prefix(repo) {
        Some(value) => json!({"value": value, "source": "repo_config"}),
        None => json!({"value": DEFAULT_ID_NAMESPACE_PREFIX, "source": "default"}),
    }
}

fn configured_scope(repo: &RepoRuntime, key: &str) -> Option<String> {
    config_text(repo, key).and_then(|value| normalize_workflow_scope_value(&value, key).ok())
}

fn configured_sprint_mode(repo: &RepoRuntime) -> Option<String> {
    config_text(repo, "sprint").and_then(|value| normalize_toggle_mode(&value, "`sprint`").ok())
}

fn sprint_mode_override(repo: &RepoRuntime) -> Option<(String, &'static str)> {
    configured_sprint_mode(repo).map(|value| (value, "repo_config"))
}

fn workflow_default_scope_summary(repo: &RepoRuntime) -> JsonValue {
    let workflow = configured_scope(repo, "workflow_default_scope")
        .map(|value| json!({"value": value, "source": "repo_config"}))
        .unwrap_or_else(|| json!({"value": DEFAULT_WORKFLOW_SCOPE, "source": "built_in"}));
    let task = configured_scope(repo, "task_default_scope")
        .map(|value| json!({"value": value, "source": "repo_config"}))
        .unwrap_or_else(|| {
            if workflow
                .get("source")
                .and_then(JsonValue::as_str)
                .is_some_and(|value| value == "repo_config")
            {
                json!({"value": workflow["value"].clone(), "source": "workflow_default_scope"})
            } else {
                json!({"value": DEFAULT_WORKFLOW_SCOPE, "source": "built_in"})
            }
        });
    let change = configured_scope(repo, "change_default_scope")
        .map(|value| json!({"value": value, "source": "repo_config"}))
        .unwrap_or_else(|| {
            if workflow
                .get("source")
                .and_then(JsonValue::as_str)
                .is_some_and(|value| value == "repo_config")
            {
                json!({"value": workflow["value"].clone(), "source": "workflow_default_scope"})
            } else {
                json!({"value": DEFAULT_WORKFLOW_SCOPE, "source": "built_in"})
            }
        });
    json!({
        "workflow": workflow,
        "task": task,
        "change": change,
    })
}

fn plan_task_binding_summary(repo: &RepoRuntime) -> JsonValue {
    if let Some((sprint, source)) = sprint_mode_override(repo) {
        return json!({
            "mode": plan_task_binding_mode_for_sprint(&sprint),
            "source": source,
        });
    }
    match configured_plan_task_binding_mode(repo) {
        Some(mode) => json!({"mode": mode, "source": "repo_config"}),
        None => json!({"mode": DEFAULT_PLAN_TASK_BINDING_MODE, "source": "staged_default"}),
    }
}

fn configured_plan_task_binding_mode(repo: &RepoRuntime) -> Option<String> {
    match repo.config.get("plan_task_binding") {
        Some(JsonValue::Object(map)) => map
            .get("mode")
            .and_then(json_text)
            .and_then(|value| normalize_stored_plan_task_binding_mode(&value).ok()),
        _ => None,
    }
}

fn sprint_summary(repo: &RepoRuntime) -> JsonValue {
    if let Some((sprint, source)) = sprint_mode_override(repo) {
        let binding_mode = plan_task_binding_mode_for_sprint(&sprint);
        let enabled = sprint == "on";
        return json!({
            "value": sprint,
            "enabled": enabled,
            "source": source,
            "plan_task_binding_mode": binding_mode,
        });
    }
    let binding = configured_plan_task_binding_mode(repo);
    let binding_mode = binding
        .as_deref()
        .unwrap_or(DEFAULT_PLAN_TASK_BINDING_MODE)
        .to_string();
    let enabled = binding_mode == "required";
    json!({
        "value": if enabled { "on" } else { "off" },
        "enabled": enabled,
        "source": if binding.is_some() { "plan_task_binding" } else { "staged_default" },
        "plan_task_binding_mode": binding_mode,
    })
}

fn workflow_mode_summary(repo: &RepoRuntime, scope_summary: &JsonValue) -> JsonValue {
    let workflow_scope = scope_summary
        .get("workflow")
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("value"))
        .and_then(JsonValue::as_str)
        .unwrap_or(DEFAULT_WORKFLOW_SCOPE);
    let task_scope = scope_summary
        .get("task")
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("value"))
        .and_then(JsonValue::as_str)
        .unwrap_or(DEFAULT_WORKFLOW_SCOPE);
    let change_scope = scope_summary
        .get("change")
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("value"))
        .and_then(JsonValue::as_str)
        .unwrap_or(DEFAULT_WORKFLOW_SCOPE);
    let binding_summary = plan_task_binding_summary(repo);
    let binding_mode = binding_summary
        .get("mode")
        .and_then(JsonValue::as_str)
        .unwrap_or(DEFAULT_PLAN_TASK_BINDING_MODE);
    if let Some(configured_mode) = config_text(repo, "workflow_mode")
        .and_then(|value| normalize_workflow_mode_value(&value, "`workflow_mode`").ok())
    {
        let preset = workflow_mode_preset(&configured_mode).unwrap();
        if workflow_scope == preset.workflow_scope
            && task_scope == preset.task_scope
            && change_scope == preset.change_scope
            && (binding_mode == preset.binding_mode || binding_mode == "off")
        {
            return json!({
                "value": configured_mode,
                "source": "repo_config",
                "change_strategy": preset.change_strategy,
            });
        }
    }
    let derived_value = if workflow_scope == "local"
        && task_scope == "local"
        && change_scope == "local"
        && matches!(binding_mode, "required" | "off")
    {
        "solo_local"
    } else if workflow_scope == "remote"
        && task_scope == "remote"
        && change_scope == "remote"
        && matches!(binding_mode, "advisory" | "off")
    {
        "solo_remote"
    } else if workflow_scope == "remote"
        && task_scope == "remote"
        && change_scope == "remote"
        && binding_mode == "required"
    {
        "team_remote"
    } else {
        "custom"
    };
    if let Some(preset) = workflow_mode_preset(derived_value) {
        return json!({
            "value": derived_value,
            "source": "derived_from_effective_config",
            "change_strategy": preset.change_strategy,
        });
    }
    json!({
        "value": "custom",
        "source": "derived_from_effective_config",
        "change_strategy": "custom",
    })
}

fn web_inbox_defaults_summary(repo: &RepoRuntime) -> JsonValue {
    let Some(JsonValue::Object(map)) = repo.config.get("web_inbox_defaults") else {
        return json!({
            "repo": JsonValue::Null,
            "author_class": JsonValue::Null,
            "author_mode": JsonValue::Null,
            "tests": JsonValue::Null,
        });
    };
    json!({
        "repo": map.get("repo").and_then(json_text),
        "author_class": map.get("author_class").and_then(json_text),
        "author_mode": map.get("author_mode").and_then(json_text),
        "tests": map.get("tests").and_then(json_text),
    })
}

fn agent_runtime_summary(repo: &RepoRuntime) -> JsonValue {
    let workflow_mode = workflow_mode_summary(repo, &workflow_default_scope_summary(repo))
        .get("value")
        .and_then(JsonValue::as_str)
        .unwrap_or("custom")
        .to_string();
    match workflow_mode.as_str() {
        "solo_local" => json!({
            "mode": "local",
            "workflow_mode": workflow_mode,
            "repo_root": repo.root.to_string_lossy().to_string(),
            "repo_name": repo.repo_name(),
            "remote_name": JsonValue::Null,
            "server_url": JsonValue::Null,
        }),
        "solo_remote" | "team_remote" => match repo.remote_row(None) {
            Ok(remote) => {
                let server_url = remote.url.trim().trim_end_matches('/').to_string();
                if server_url.is_empty() {
                    json!({"error": "The default ait remote is missing a server URL."})
                } else {
                    json!({
                        "mode": "remote",
                        "workflow_mode": workflow_mode,
                        "repo_root": repo.root.to_string_lossy().to_string(),
                        "repo_name": repo.repo_name(),
                        "remote_name": Some(remote.name),
                        "server_url": server_url,
                    })
                }
            }
            Err(err) => json!({"error": err}),
        },
        _ => json!({
            "error": "ait-agent requires a repo workflow preset. Set `ait config set --workflow-mode solo_local|solo_remote|team_remote` before starting agent workers."
        }),
    }
}

fn workflow_mode_preset(value: &str) -> Option<WorkflowModePreset> {
    match value {
        "solo_local" => Some(WorkflowModePreset {
            workflow_scope: "local",
            task_scope: "local",
            change_scope: "local",
            binding_mode: "required",
            change_strategy: "promote_reviewable_outputs_late",
        }),
        "solo_remote" => Some(WorkflowModePreset {
            workflow_scope: "remote",
            task_scope: "remote",
            change_scope: "remote",
            binding_mode: "required",
            change_strategy: "remote_backed_selective_promotion",
        }),
        "team_remote" => Some(WorkflowModePreset {
            workflow_scope: "remote",
            task_scope: "remote",
            change_scope: "remote",
            binding_mode: "required",
            change_strategy: "per_slice_reviewable_changes",
        }),
        _ => None,
    }
}

struct WorkflowModePreset {
    workflow_scope: &'static str,
    task_scope: &'static str,
    change_scope: &'static str,
    binding_mode: &'static str,
    change_strategy: &'static str,
}

fn option_string_field(
    object: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<String>, String> {
    match object.get(key) {
        None => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!(
            "config set payload field `{key}` must be a string."
        )),
    }
}

fn config_scalar_text(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => normalize_text(Some(text)),
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn option_i64_field(object: &JsonMap<String, JsonValue>, key: &str) -> Result<Option<i64>, String> {
    match object.get(key) {
        None => Ok(None),
        Some(JsonValue::Number(value)) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("config set payload field `{key}` must be an integer.")),
        Some(_) => Err(format!(
            "config set payload field `{key}` must be an integer."
        )),
    }
}

fn read_json_object(path: &Path) -> JsonMap<String, JsonValue> {
    let Ok(content) = fs::read_to_string(path) else {
        return JsonMap::new();
    };
    parse_object_or_empty(&content)
}

fn write_json_pretty(path: &Path, payload: &JsonValue) -> Result<(), String> {
    let encoded = encode_value_pretty_with_newline_error_string(payload)?;
    fs::write(path, encoded).map_err(|err| err.to_string())
}

fn update_root_config(
    repo: &RepoRuntime,
    updater: impl FnOnce(&mut JsonMap<String, JsonValue>) -> Result<(), String>,
) -> Result<(), String> {
    let config_path = repo
        .authoritative_repo_root()
        .join(".ait")
        .join("config.json");
    let mut config = read_json_object(&config_path);
    updater(&mut config)?;
    write_json_pretty(&config_path, &JsonValue::Object(config))
}

#[cfg(test)]
mod tests;
