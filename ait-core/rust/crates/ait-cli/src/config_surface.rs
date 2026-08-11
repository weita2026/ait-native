use crate::agent_harness::converge_agent_workflow_harness;
use crate::json_support::{encode_value_pretty_with_newline_error_string, parse_object_or_empty};
use crate::runtime::RepoRuntime;
use crate::task_worktree_layout::{auto_detected_ephemeral_root, config_task_worktree_summary};
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::server_operational::{
    RepositoryIndex, ServerRepositoryAuthorityConfig, REPOSITORY_INDEX_CONFIG_KEY,
};
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
    pub repository_index: Option<u32>,
    pub clear_repository_index: bool,
    pub default_author_mode: Option<String>,
    pub clear_default_author_mode: bool,
    pub default_model: Option<String>,
    pub clear_default_model: bool,
    pub task_tracking: Option<String>,
    pub task_review: Option<String>,
    pub command_profiling: Option<String>,
    pub task_worktree_alias_root: Option<String>,
    pub clear_task_worktree_alias_root: bool,
    pub task_worktree_main_seed_ram_max_bytes: Option<i64>,
    pub clear_task_worktree_main_seed_ram_max_bytes: bool,
    pub legacy_task_auto_worktree: Option<String>,
    pub legacy_clear_task_auto_worktree: bool,
    pub workflow_mode: Option<String>,
    pub workflow_default_scope: Option<String>,
    pub clear_workflow_default_scope: bool,
    pub task_default_scope: Option<String>,
    pub clear_task_default_scope: bool,
    pub change_default_scope: Option<String>,
    pub clear_change_default_scope: bool,
    pub id_namespace_prefix: Option<String>,
    pub clear_id_namespace_prefix: bool,
    pub sprint: Option<String>,
    pub plan_task_binding_mode: Option<String>,
    pub clear_plan_task_binding: bool,
    pub user_name: Option<String>,
    pub clear_user_name: bool,
    pub user_email: Option<String>,
    pub clear_user_email: bool,
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
        "detected_model": detected_model(),
        "detected_actor": detected_actor(),
        "user_name": config_text(repo, "user_name"),
        "user_email": config_text(repo, "user_email"),
        "effective_actor": effective_actor(repo),
        "effective_reviewer": effective_reviewer(repo),
        "effective_author_mode": effective_author_mode(repo),
        "effective_model": effective_model(repo),
        "task_tracking": task_tracking_mode(repo),
        "task_review": task_review_summary(repo),
        "command_profiling": command_profiling_mode(repo),
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
    update_root_config(repo, |config| {
        apply_config_set_updates(repo, config, request)
    })?;
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
    Ok(ConfigSetRequest {
        repository_index: option_u32_field(object, REPOSITORY_INDEX_CONFIG_KEY)?,
        clear_repository_index: bool_field(object, "clear_repository_index")?,
        default_author_mode: option_string_field(object, "default_author_mode")?,
        clear_default_author_mode: bool_field(object, "clear_default_author_mode")?,
        default_model: option_string_field(object, "default_model")?,
        clear_default_model: bool_field(object, "clear_default_model")?,
        task_tracking: option_string_field(object, "task_tracking")?,
        task_review: option_string_field(object, "task_review")?,
        command_profiling: option_string_field(object, "command_profiling")?,
        task_worktree_alias_root: option_string_field(object, "task_worktree_alias_root")?,
        clear_task_worktree_alias_root: bool_field(object, "clear_task_worktree_alias_root")?,
        task_worktree_main_seed_ram_max_bytes: option_i64_field(
            object,
            "task_worktree_main_seed_ram_max_bytes",
        )?,
        clear_task_worktree_main_seed_ram_max_bytes: bool_field(
            object,
            "clear_task_worktree_main_seed_ram_max_bytes",
        )?,
        legacy_task_auto_worktree: option_string_field(object, "legacy_task_auto_worktree")?,
        legacy_clear_task_auto_worktree: bool_field(object, "legacy_clear_task_auto_worktree")?,
        workflow_mode: option_string_field(object, "workflow_mode")?,
        workflow_default_scope: option_string_field(object, "workflow_default_scope")?,
        clear_workflow_default_scope: bool_field(object, "clear_workflow_default_scope")?,
        task_default_scope: option_string_field(object, "task_default_scope")?,
        clear_task_default_scope: bool_field(object, "clear_task_default_scope")?,
        change_default_scope: option_string_field(object, "change_default_scope")?,
        clear_change_default_scope: bool_field(object, "clear_change_default_scope")?,
        id_namespace_prefix: option_string_field(object, "id_namespace_prefix")?,
        clear_id_namespace_prefix: bool_field(object, "clear_id_namespace_prefix")?,
        sprint: option_string_field(object, "sprint")?,
        plan_task_binding_mode: option_string_field(object, "plan_task_binding_mode")?,
        clear_plan_task_binding: bool_field(object, "clear_plan_task_binding")?,
        user_name: option_string_field(object, "user_name")?,
        clear_user_name: bool_field(object, "clear_user_name")?,
        user_email: option_string_field(object, "user_email")?,
        clear_user_email: bool_field(object, "clear_user_email")?,
    })
}

fn validate_config_set_request(request: &ConfigSetRequest) -> Result<(), String> {
    ensure_exclusive(
        request.repository_index.as_ref(),
        request.clear_repository_index,
        "--repository-index",
        "--clear-repository-index",
    )?;
    ensure_exclusive(
        request.default_author_mode.as_ref(),
        request.clear_default_author_mode,
        "--default-author-mode",
        "--clear-default-author-mode",
    )?;
    ensure_exclusive(
        request.default_model.as_ref(),
        request.clear_default_model,
        "--default-model",
        "--clear-default-model",
    )?;
    ensure_exclusive(
        request.legacy_task_auto_worktree.as_ref(),
        request.legacy_clear_task_auto_worktree,
        "--task-auto-worktree",
        "--clear-task-auto-worktree",
    )?;
    ensure_exclusive(
        request.task_worktree_alias_root.as_ref(),
        request.clear_task_worktree_alias_root,
        "--task-worktree-alias-root",
        "--clear-task-worktree-alias-root",
    )?;
    ensure_exclusive(
        request.task_worktree_main_seed_ram_max_bytes.as_ref(),
        request.clear_task_worktree_main_seed_ram_max_bytes,
        "--task-worktree-main-seed-ram-max-bytes",
        "--clear-task-worktree-main-seed-ram-max-bytes",
    )?;
    ensure_exclusive(
        request.id_namespace_prefix.as_ref(),
        request.clear_id_namespace_prefix,
        "--id-namespace-prefix",
        "--clear-id-namespace-prefix",
    )?;
    ensure_exclusive(
        request.workflow_default_scope.as_ref(),
        request.clear_workflow_default_scope,
        "--workflow-default-scope",
        "--clear-workflow-default-scope",
    )?;
    ensure_exclusive(
        request.task_default_scope.as_ref(),
        request.clear_task_default_scope,
        "--task-default-scope",
        "--clear-task-default-scope",
    )?;
    ensure_exclusive(
        request.change_default_scope.as_ref(),
        request.clear_change_default_scope,
        "--change-default-scope",
        "--clear-change-default-scope",
    )?;
    ensure_exclusive(
        request.user_name.as_ref(),
        request.clear_user_name,
        "--user-name",
        "--clear-user-name",
    )?;
    ensure_exclusive(
        request.user_email.as_ref(),
        request.clear_user_email,
        "--user-email",
        "--clear-user-email",
    )?;
    if request.sprint.is_some() && request.plan_task_binding_mode.is_some() {
        return Err("Choose either --sprint or --plan-task-binding-mode.".to_string());
    }
    if request.sprint.is_some() && request.clear_plan_task_binding {
        return Err("Choose either --sprint or --clear-plan-task-binding.".to_string());
    }
    if request.clear_plan_task_binding && request.plan_task_binding_mode.is_some() {
        return Err(
            "Choose either --clear-plan-task-binding or explicit --plan-task-binding-* updates"
                .to_string(),
        );
    }
    if request.workflow_mode.is_some()
        && (request.workflow_default_scope.is_some()
            || request.task_default_scope.is_some()
            || request.change_default_scope.is_some()
            || request.plan_task_binding_mode.is_some()
            || request.clear_workflow_default_scope
            || request.clear_task_default_scope
            || request.clear_change_default_scope
            || request.clear_plan_task_binding)
    {
        return Err(
            "`--workflow-mode` cannot be combined with manual workflow scope or plan/task binding overrides.".to_string(),
        );
    }
    if !request_has_updates(request) {
        return Err("No config updates specified".to_string());
    }
    Ok(())
}

fn request_has_updates(request: &ConfigSetRequest) -> bool {
    request.repository_index.is_some()
        || request.clear_repository_index
        || request.default_author_mode.is_some()
        || request.clear_default_author_mode
        || request.default_model.is_some()
        || request.clear_default_model
        || request.task_tracking.is_some()
        || request.task_review.is_some()
        || request.command_profiling.is_some()
        || request.task_worktree_alias_root.is_some()
        || request.clear_task_worktree_alias_root
        || request.task_worktree_main_seed_ram_max_bytes.is_some()
        || request.clear_task_worktree_main_seed_ram_max_bytes
        || request.legacy_task_auto_worktree.is_some()
        || request.legacy_clear_task_auto_worktree
        || request.workflow_mode.is_some()
        || request.workflow_default_scope.is_some()
        || request.clear_workflow_default_scope
        || request.task_default_scope.is_some()
        || request.clear_task_default_scope
        || request.change_default_scope.is_some()
        || request.clear_change_default_scope
        || request.id_namespace_prefix.is_some()
        || request.clear_id_namespace_prefix
        || request.sprint.is_some()
        || request.plan_task_binding_mode.is_some()
        || request.clear_plan_task_binding
        || request.user_name.is_some()
        || request.clear_user_name
        || request.user_email.is_some()
        || request.clear_user_email
}

fn ensure_exclusive<T>(
    value: Option<&T>,
    clear_flag: bool,
    set_flag: &str,
    clear_name: &str,
) -> Result<(), String> {
    if value.is_some() && clear_flag {
        return Err(format!("Choose either {set_flag} or {clear_name}"));
    }
    Ok(())
}

fn apply_config_set_updates(
    repo: &RepoRuntime,
    config: &mut JsonMap<String, JsonValue>,
    request: &ConfigSetRequest,
) -> Result<(), String> {
    if request.clear_repository_index {
        config.remove(REPOSITORY_INDEX_CONFIG_KEY);
    } else if let Some(value) = request.repository_index {
        config.insert(
            REPOSITORY_INDEX_CONFIG_KEY.to_string(),
            JsonValue::Number(value.into()),
        );
    }

    if let Some(value) = request.default_author_mode.as_ref() {
        config.insert(
            "default_author_mode".to_string(),
            JsonValue::String(normalize_author_mode_value(value)?),
        );
    } else if request.clear_default_author_mode {
        config.remove("default_author_mode");
    }

    if let Some(value) = request.default_model.as_ref() {
        match normalize_text(Some(value)) {
            Some(model) => {
                config.insert("default_model".to_string(), JsonValue::String(model));
            }
            None => {
                config.remove("default_model");
            }
        }
    } else if request.clear_default_model {
        config.remove("default_model");
    }

    if let Some(value) = request.task_tracking.as_ref() {
        let normalized = normalize_toggle_mode(value, "`--task-tracking`")?;
        config.insert(
            "task_tracking".to_string(),
            JsonValue::String(normalized.clone()),
        );
    }

    if let Some(value) = request.task_review.as_ref() {
        config.insert(
            "task_review".to_string(),
            JsonValue::Bool(normalize_bool_toggle(value, "`--task-review`")?),
        );
    }

    if let Some(value) = request.command_profiling.as_ref() {
        config.insert(
            "command_profiling".to_string(),
            JsonValue::String(normalize_toggle_mode(value, "`--command-profiling`")?),
        );
    }

    if let Some(value) = request.legacy_task_auto_worktree.as_ref() {
        let _ = normalize_toggle_mode(value, "`--task-auto-worktree`")?;
    }

    let mut task_worktree_config = normalized_task_worktree_config(config.get("task_worktree"));
    task_worktree_config.remove("auto_remove_after_remote_land");
    task_worktree_config.remove("root_mode");
    if request.clear_task_worktree_alias_root {
        task_worktree_config.remove("alias_root");
    } else if let Some(value) = request.task_worktree_alias_root.as_ref() {
        match normalize_text(Some(value)) {
            Some(alias_root) => {
                task_worktree_config
                    .insert("alias_root".to_string(), JsonValue::String(alias_root));
            }
            None => {
                task_worktree_config.remove("alias_root");
            }
        }
    }
    if request.clear_task_worktree_main_seed_ram_max_bytes {
        task_worktree_config.remove("main_seed_ram_max_bytes");
    } else if let Some(value) = request.task_worktree_main_seed_ram_max_bytes {
        if value < 0 {
            return Err(
                "`--task-worktree-main-seed-ram-max-bytes` must be a non-negative integer."
                    .to_string(),
            );
        }
        task_worktree_config.insert(
            "main_seed_ram_max_bytes".to_string(),
            JsonValue::Number(value.into()),
        );
    }
    if let Some(derived_root) = derived_task_worktree_ephemeral_root(repo, &task_worktree_config) {
        let stored = task_worktree_config
            .get("ephemeral_root")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalize_text(Some(value)))
            .map(|raw| resolve_configured_path(&repo.authoritative_repo_root(), &raw));
        if stored.as_ref() == Some(&derived_root) {
            task_worktree_config.remove("ephemeral_root");
        }
    }
    if task_worktree_config.is_empty() {
        config.remove("task_worktree");
    } else {
        config.insert(
            "task_worktree".to_string(),
            JsonValue::Object(task_worktree_config),
        );
    }

    if let Some(value) = request.workflow_mode.as_ref() {
        let normalized = normalize_workflow_mode_value(value, "--workflow-mode")?;
        let sprint = request
            .sprint
            .as_ref()
            .map(|value| normalize_toggle_mode(value, "`--sprint`"))
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
    } else {
        if request.clear_workflow_default_scope
            || request.workflow_default_scope.is_some()
            || request.clear_task_default_scope
            || request.task_default_scope.is_some()
            || request.clear_change_default_scope
            || request.change_default_scope.is_some()
            || request.clear_plan_task_binding
            || request.plan_task_binding_mode.is_some()
        {
            config.remove("workflow_mode");
        }
        if request.clear_workflow_default_scope {
            config.remove("workflow_default_scope");
        } else if let Some(value) = request.workflow_default_scope.as_ref() {
            config.insert(
                "workflow_default_scope".to_string(),
                JsonValue::String(normalize_workflow_scope_value(
                    value,
                    "--workflow-default-scope",
                )?),
            );
        }
        if request.clear_task_default_scope {
            config.remove("task_default_scope");
        } else if let Some(value) = request.task_default_scope.as_ref() {
            config.insert(
                "task_default_scope".to_string(),
                JsonValue::String(normalize_workflow_scope_value(
                    value,
                    "--task-default-scope",
                )?),
            );
        }
        if request.clear_change_default_scope {
            config.remove("change_default_scope");
        } else if let Some(value) = request.change_default_scope.as_ref() {
            config.insert(
                "change_default_scope".to_string(),
                JsonValue::String(normalize_workflow_scope_value(
                    value,
                    "--change-default-scope",
                )?),
            );
        }
    }

    if request.clear_id_namespace_prefix {
        config.remove("id_namespace_prefix");
    } else if let Some(value) = request.id_namespace_prefix.as_ref() {
        config.insert(
            "id_namespace_prefix".to_string(),
            JsonValue::String(normalize_id_namespace_prefix_value(value)?),
        );
    }

    if request.workflow_mode.is_none() {
        if let Some(value) = request.sprint.as_ref() {
            let sprint = normalize_toggle_mode(value, "`--sprint`")?;
            config.insert("sprint".to_string(), JsonValue::String(sprint.clone()));
            config.insert(
                "plan_task_binding".to_string(),
                json!({"mode": plan_task_binding_mode_for_sprint(&sprint)}),
            );
        } else if request.clear_plan_task_binding {
            config.remove("plan_task_binding");
            config.remove("sprint");
        } else if let Some(value) = request.plan_task_binding_mode.as_ref() {
            config.insert(
                "plan_task_binding".to_string(),
                json!({"mode": normalize_plan_task_binding_mode_value(value)?}),
            );
            config.remove("sprint");
        }
    }

    if let Some(value) = request.user_name.as_ref() {
        match normalize_text(Some(value)) {
            Some(user_name) => {
                config.insert("user_name".to_string(), JsonValue::String(user_name));
            }
            None => {
                config.remove("user_name");
            }
        }
    } else if request.clear_user_name {
        config.remove("user_name");
    }

    if let Some(value) = request.user_email.as_ref() {
        match normalize_text(Some(value)) {
            Some(user_email) => {
                config.insert("user_email".to_string(), JsonValue::String(user_email));
            }
            None => {
                config.remove("user_email");
            }
        }
    } else if request.clear_user_email {
        config.remove("user_email");
    }

    for key in [
        "workflow_default_scope",
        "task_default_scope",
        "change_default_scope",
    ] {
        if config
            .get(key)
            .and_then(JsonValue::as_str)
            .and_then(|value| normalize_text(Some(value)))
            .is_none()
        {
            config.remove(key);
        }
    }
    if config
        .get("id_namespace_prefix")
        .and_then(JsonValue::as_str)
        .is_some_and(|value| value.trim().is_empty())
    {
        config.insert(
            "id_namespace_prefix".to_string(),
            JsonValue::String(String::new()),
        );
    }
    Ok(())
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
    match normalize_text(Some(value))
        .unwrap_or_default()
        .as_str()
    {
        "human_only" | "human_with_ai_assist" | "ai_with_human_review"
        | "ai_only_experimental" => Ok(value.trim().to_string()),
        _ => Err(
            "Unknown author_mode. Expected one of: human_only, human_with_ai_assist, ai_with_human_review, ai_only_experimental"
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

fn normalize_bool_toggle(value: &str, option_name: &str) -> Result<bool, String> {
    match normalize_text(Some(value))
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err(format!("{option_name} must be `on` or `off`.")),
    }
}

fn normalize_plan_task_binding_mode_value(value: &str) -> Result<String, String> {
    match normalize_text(Some(value))
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "off" | "advisory" | "strict" | "required" => Ok(value.trim().to_lowercase()),
        _ => Err(
            "`--plan-task-binding-mode` must be `off`, `advisory`, `strict`, or `required`."
                .to_string(),
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

fn detected_model() -> Option<String> {
    for key in ["AIT_MODEL", "CODEX_MODEL", "OPENAI_MODEL"] {
        if let Ok(value) = env::var(key) {
            if let Some(normalized) = normalize_text(Some(&value)) {
                return Some(normalized);
            }
        }
    }
    None
}

fn detected_actor() -> Option<String> {
    env::var("AIT_NATIVE_ACTOR")
        .ok()
        .and_then(|value| normalize_text(Some(&value)))
        .or_else(|| {
            env::var("AIT_ACTOR")
                .ok()
                .and_then(|value| normalize_text(Some(&value)))
        })
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
    detected_model().or_else(|| config_text(repo, "default_model"))
}

fn task_tracking_mode(repo: &RepoRuntime) -> Option<String> {
    config_text(repo, "task_tracking")
        .and_then(|value| normalize_toggle_mode(&value, "`task_tracking`").ok())
}

fn task_review_summary(repo: &RepoRuntime) -> JsonValue {
    match configured_task_review(repo) {
        Some(value) => json!({"value": value, "source": "repo_config"}),
        None => json!({"value": false, "source": "built_in"}),
    }
}

fn configured_task_review(repo: &RepoRuntime) -> Option<bool> {
    if !repo.config.contains_key("task_review") {
        return None;
    }
    repo.config
        .get("task_review")
        .and_then(|value| match value {
            JsonValue::Bool(flag) => Some(*flag),
            JsonValue::String(text) => normalize_bool_toggle(text, "`task_review`").ok(),
            _ => None,
        })
}

fn command_profiling_mode(repo: &RepoRuntime) -> String {
    config_text(repo, "command_profiling")
        .and_then(|value| normalize_toggle_mode(&value, "`command_profiling`").ok())
        .unwrap_or_else(|| "off".to_string())
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

fn detected_sprint_mode() -> Option<String> {
    for key in ["AIT_SPRINT", "AIT_ENABLE_SPRINT"] {
        if let Ok(value) = env::var(key) {
            if let Ok(mode) = normalize_toggle_mode(&value, key) {
                return Some(mode);
            }
        }
    }
    None
}

fn configured_sprint_mode(repo: &RepoRuntime) -> Option<String> {
    config_text(repo, "sprint").and_then(|value| normalize_toggle_mode(&value, "`sprint`").ok())
}

fn sprint_mode_override(repo: &RepoRuntime) -> Option<(String, &'static str)> {
    detected_sprint_mode()
        .map(|value| (value, "env"))
        .or_else(|| configured_sprint_mode(repo).map(|value| (value, "repo_config")))
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
            .and_then(|value| normalize_plan_task_binding_mode_value(&value).ok()),
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

fn normalized_task_worktree_config(value: Option<&JsonValue>) -> JsonMap<String, JsonValue> {
    let mut out = JsonMap::new();
    let Some(JsonValue::Object(raw)) = value else {
        return out;
    };
    if let Some(ephemeral_root) = raw
        .get("ephemeral_root")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalize_text(Some(value)))
    {
        out.insert(
            "ephemeral_root".to_string(),
            JsonValue::String(ephemeral_root),
        );
    }
    if let Some(alias_root) = raw
        .get("alias_root")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalize_text(Some(value)))
    {
        out.insert("alias_root".to_string(), JsonValue::String(alias_root));
    }
    if let Some(memory_root) = raw.get("memory_root").and_then(normalized_memory_root_json) {
        out.insert("memory_root".to_string(), memory_root);
    }
    if let Some(main_seed_ram_max_bytes) = raw
        .get("main_seed_ram_max_bytes")
        .and_then(json_i64)
        .filter(|value| *value >= 0)
    {
        out.insert(
            "main_seed_ram_max_bytes".to_string(),
            JsonValue::Number(main_seed_ram_max_bytes.into()),
        );
    }
    out
}

fn normalized_memory_root_json(value: &JsonValue) -> Option<JsonValue> {
    let JsonValue::Object(raw) = value else {
        return None;
    };
    let kind = raw.get("kind").and_then(json_text)?;
    let root = raw.get("root").and_then(json_text)?;
    let normalized_kind = match kind.as_str() {
        "macos_ram_volume" | "linux_memory_root" | "windows_ramdisk" => kind,
        _ => return None,
    };
    let mut out = JsonMap::new();
    out.insert("kind".to_string(), JsonValue::String(normalized_kind));
    out.insert("root".to_string(), JsonValue::String(root));
    if let Some(volume_name) = raw.get("volume_name").and_then(json_text) {
        out.insert("volume_name".to_string(), JsonValue::String(volume_name));
    }
    if let Some(sector_count) = raw.get("sector_count").and_then(json_i64) {
        out.insert(
            "sector_count".to_string(),
            JsonValue::Number(sector_count.into()),
        );
    }
    Some(JsonValue::Object(out))
}

fn derived_task_worktree_ephemeral_root(
    repo: &RepoRuntime,
    task_worktree_config: &JsonMap<String, JsonValue>,
) -> Option<std::path::PathBuf> {
    let memory_root = task_worktree_config
        .get("memory_root")
        .and_then(normalized_memory_root_json)
        .and_then(|value| match value {
            JsonValue::Object(map) => map.get("root").and_then(json_text),
            _ => None,
        })?;
    Some(auto_detected_ephemeral_root(repo, Path::new(&memory_root)))
}

fn resolve_configured_path(repo_root: &Path, value: &str) -> std::path::PathBuf {
    let configured = std::path::PathBuf::from(value);
    if configured.is_absolute() {
        configured
    } else {
        repo_root.join(configured)
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
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(text.clone())),
        Some(JsonValue::Number(number)) => Ok(Some(number.to_string())),
        Some(JsonValue::Bool(flag)) => Ok(Some(flag.to_string())),
        Some(_) => Err(format!(
            "config set payload field `{key}` must be scalar text."
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
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => json_i64(value)
            .map(Some)
            .ok_or_else(|| format!("config set payload field `{key}` must be an integer.")),
    }
}

fn option_u32_field(object: &JsonMap<String, JsonValue>, key: &str) -> Result<Option<u32>, String> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    RepositoryIndex::parse_config_value(value)
        .map(|value| Some(value.get()))
        .map_err(|error| format!("config set payload field `{key}` is invalid: {error}"))
}

fn bool_field(object: &JsonMap<String, JsonValue>, key: &str) -> Result<bool, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(false),
        Some(JsonValue::Bool(flag)) => Ok(*flag),
        Some(JsonValue::String(text)) => match text.trim().to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(format!("config set payload field `{key}` must be boolean.")),
        },
        Some(_) => Err(format!("config set payload field `{key}` must be boolean.")),
    }
}

fn json_i64(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::String(text) => normalize_text(Some(text)).and_then(|value| value.parse().ok()),
        _ => None,
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
