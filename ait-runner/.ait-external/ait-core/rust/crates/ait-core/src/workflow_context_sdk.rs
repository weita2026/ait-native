//! Shared retained workflow projection context and native hint behavior.
//! The host interface reads context metadata only; N-API uses owned Rust data.
use crate::json_support::{JsonCodec, JsonEncodeOptions};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowContextError {
    Invalid(String),
    MissingKey(String),
    Host(String),
}
impl WorkflowContextError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
    pub fn missing_key(message: impl Into<String>) -> Self {
        Self::MissingKey(message.into())
    }
}
impl std::fmt::Display for WorkflowContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) | Self::MissingKey(message) | Self::Host(message) => {
                f.write_str(message)
            }
        }
    }
}
impl std::error::Error for WorkflowContextError {}
pub type WorkflowContextResult<T> = Result<T, WorkflowContextError>;

/// Existing context metadata used by workflow projection/hint services.
pub trait WorkflowContext {
    fn bool_toggle(&self, attr_name: &str) -> WorkflowContextResult<Option<bool>>;
    fn path(&self, attr_name: &str) -> WorkflowContextResult<Option<PathBuf>>;
}
#[derive(Clone, Debug, Default)]
pub struct WorkflowContextData {
    pub root: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub worktree_config_path: Option<PathBuf>,
    pub workspace_dir: Option<PathBuf>,
    pub task_review: Option<bool>,
}
impl WorkflowContext for WorkflowContextData {
    fn bool_toggle(&self, attr_name: &str) -> WorkflowContextResult<Option<bool>> {
        Ok(if attr_name == "task_review" {
            self.task_review
        } else {
            None
        })
    }
    fn path(&self, attr_name: &str) -> WorkflowContextResult<Option<PathBuf>> {
        Ok(match attr_name {
            "root" => self.root.clone(),
            "config_path" => self.config_path.clone(),
            "worktree_config_path" => self.worktree_config_path.clone(),
            "workspace_dir" => self.workspace_dir.clone(),
            _ => None,
        })
    }
}
fn encode_json_value_pretty(value: &JsonValue) -> Result<String, String> {
    JsonCodec::encode_value(value, JsonEncodeOptions::pretty()).map_err(|error| {
        error
            .message()
            .strip_prefix("Failed to encode JSON: ")
            .unwrap_or(error.message())
            .to_string()
    })
}
fn parse_json_object_or_empty(text: &str) -> Map<String, JsonValue> {
    match JsonCodec::parse_value_with_error_prefix(text, "Invalid JSON") {
        Ok(JsonValue::Object(object)) => object,
        _ => Map::new(),
    }
}
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::environment_contract::names;
use crate::json_support::{json, JsonMap as Map, JsonValue};
use crate::plan_http_client::{PlanHttpClientConfig, PlanHttpClientManager};
use crate::remote_store::{remote_by_name_with_remote_store, ConfigRemoteStore};
use chrono::{DateTime, FixedOffset, Utc};

const DEFAULT_TASK_REVIEW: bool = false;
const WORKFLOW_READY_POLL_SECONDS_KEY: &str = "workflow_ready_poll_seconds";
const WORKFLOW_LAND_POLL_SECONDS_KEY: &str = "workflow_land_poll_seconds";
const WORKFLOW_WAIT_HINT_BOOTSTRAP_MISS: i64 = 0;
const WORKFLOW_WAIT_HINT_ALPHA: f64 = 0.5;
const WORKFLOW_WAIT_HINT_MIN_SECONDS: i64 = 5;
const WORKFLOW_WAIT_HINT_MAX_SECONDS: i64 = 900;
const WORKFLOW_WAIT_HINT_HISTORY_LIMIT: usize = 40;
const WORKFLOW_WAIT_HINT_SAMPLE_LIMIT: usize = 12;
const CODE_REVIEW_SUMMARY_TEMPLATE: &str =
    "Reviewed files: <paths reviewed>; Findings: <findings resolved before submission>; Risks: <residual risks>; Tests: <checks run>; Recommendation: <pass when this exact Patchset is ready>";
const CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND: &str = "ait review code template --style numbered";

fn normalize_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_json_text(value: Option<&JsonValue>) -> Option<String> {
    normalize_text(value.and_then(JsonValue::as_str))
}

fn json_bool_toggle(value: Option<&JsonValue>) -> Option<bool> {
    match value {
        Some(JsonValue::Bool(value)) => Some(*value),
        Some(JsonValue::String(text)) => match text.trim().to_ascii_lowercase().as_str() {
            "on" | "true" | "yes" | "1" => Some(true),
            "off" | "false" | "no" | "0" => Some(false),
            _ => None,
        },
        Some(JsonValue::Number(value)) => value.as_i64().map(|numeric| numeric != 0),
        _ => None,
    }
}

fn ctx_bool_toggle(
    ctx: &dyn WorkflowContext,
    attr_name: &str,
) -> WorkflowContextResult<Option<bool>> {
    ctx.bool_toggle(attr_name)
}

fn ctx_path(ctx: &dyn WorkflowContext, attr_name: &str) -> WorkflowContextResult<Option<PathBuf>> {
    ctx.path(attr_name)
}

fn ctx_required_path(ctx: &dyn WorkflowContext, attr_name: &str) -> WorkflowContextResult<PathBuf> {
    ctx_path(ctx, attr_name)?.ok_or_else(|| {
        WorkflowContextError::invalid(format!("Task/workflow context must expose {attr_name}."))
    })
}

fn read_json_object(path: &Path) -> Map<String, JsonValue> {
    let Ok(text) = fs::read_to_string(path) else {
        return Map::new();
    };
    parse_json_object_or_empty(&text)
}

fn merged_config(ctx: &dyn WorkflowContext) -> WorkflowContextResult<Map<String, JsonValue>> {
    let mut merged = match ctx_path(ctx, "config_path")? {
        Some(config_path) => read_json_object(&config_path),
        None => Map::new(),
    };
    if let Some(worktree_config_path) = ctx_path(ctx, "worktree_config_path")? {
        let overlay = read_json_object(&worktree_config_path);
        for (key, value) in overlay {
            if !value.is_null() {
                merged.insert(key, value);
            }
        }
    }
    Ok(merged)
}

fn base_config_path(ctx: &dyn WorkflowContext) -> WorkflowContextResult<PathBuf> {
    ctx_required_path(ctx, "config_path")
}

fn update_base_config_value(
    ctx: &dyn WorkflowContext,
    key: &str,
    value: JsonValue,
) -> WorkflowContextResult<()> {
    let config_path = base_config_path(ctx)?;
    let mut base = read_json_object(&config_path);
    base.insert(key.to_string(), value);
    let payload = encode_json_value_pretty(&JsonValue::Object(base))
        .map_err(WorkflowContextError::invalid)?;
    fs::write(config_path, payload)
        .map_err(|err| WorkflowContextError::invalid(err.to_string()))?;
    Ok(())
}

fn remove_base_config_value(ctx: &dyn WorkflowContext, key: &str) -> WorkflowContextResult<()> {
    let config_path = base_config_path(ctx)?;
    let mut base = read_json_object(&config_path);
    base.remove(key);
    let payload = encode_json_value_pretty(&JsonValue::Object(base))
        .map_err(WorkflowContextError::invalid)?;
    fs::write(config_path, payload)
        .map_err(|err| WorkflowContextError::invalid(err.to_string()))?;
    Ok(())
}

pub fn effective_task_review(ctx: &dyn WorkflowContext) -> WorkflowContextResult<JsonValue> {
    if let Some(value) = ctx_bool_toggle(ctx, "task_review")? {
        return Ok(json!({"value": value}));
    }
    let config = merged_config(ctx)?;
    if config.contains_key("task_review") {
        if let Some(value) = json_bool_toggle(config.get("task_review")) {
            return Ok(json!({"value": value, "source": "repo_config"}));
        }
    }
    Ok(json!({"value": DEFAULT_TASK_REVIEW, "source": "built_in"}))
}

pub fn task_review_enabled(ctx: &dyn WorkflowContext) -> WorkflowContextResult<bool> {
    Ok(effective_task_review(ctx)?
        .get("value")
        .and_then(JsonValue::as_bool)
        .unwrap_or(DEFAULT_TASK_REVIEW))
}

fn team_review_enabled(config: &Map<String, JsonValue>) -> bool {
    config
        .get("agent_teams")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn lookup_remote_row(
    config_path: &Path,
    name: &str,
) -> WorkflowContextResult<Map<String, JsonValue>> {
    let store = ConfigRemoteStore::new(config_path).map_err(WorkflowContextError::invalid)?;
    let remote = remote_by_name_with_remote_store(&store, name)
        .map_err(WorkflowContextError::invalid)?
        .ok_or_else(|| WorkflowContextError::missing_key(format!("Unknown remote: {name}")))?;
    let mut payload = Map::new();
    payload.insert("remote_id".to_string(), json!(remote.remote_id));
    payload.insert("name".to_string(), json!(remote.name));
    payload.insert("url".to_string(), json!(remote.url));
    payload.insert(
        "repo_name".to_string(),
        remote
            .repo_name
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    payload.insert("is_default_push".to_string(), json!(remote.is_default_push));
    payload.insert("is_default_pull".to_string(), json!(remote.is_default_pull));
    payload.insert("created_at".to_string(), json!(remote.created_at));
    Ok(payload)
}

fn remote_tuple(
    ctx: &dyn WorkflowContext,
    remote_name: Option<&str>,
) -> WorkflowContextResult<(Map<String, JsonValue>, String)> {
    let config = merged_config(ctx)?;
    let resolved_remote_name = normalize_text(remote_name).or_else(|| {
        config
            .get("default_remote")
            .and_then(JsonValue::as_str)
            .map(str::to_string)
    });
    let Some(resolved_remote_name) = resolved_remote_name else {
        return Err(WorkflowContextError::missing_key(
            "No remote configured. Run `ait remote add ... --default` first.",
        ));
    };
    let config_path = base_config_path(ctx)?;
    let remote = lookup_remote_row(&config_path, &resolved_remote_name)?;
    let repo_name = normalize_json_text(remote.get("repo_name"))
        .or_else(|| {
            config
                .get("repo_name")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| WorkflowContextError::missing_key("Remote context is missing repo_name."))?;
    Ok((remote, repo_name))
}

fn task_review_reviewer_identity(config: &Map<String, JsonValue>) -> Option<String> {
    normalize_json_text(config.get("user_name"))
}

fn local_patchset_ci_contract_exists(root: &Path) -> bool {
    local_patchset_ci_catalog_path(root).is_some()
}

fn local_patchset_ci_catalog_path(root: &Path) -> Option<PathBuf> {
    let default_catalog = root.join("ci").join("patch_ci.json");
    if default_catalog.is_file() {
        return Some(default_catalog);
    }

    let contract = read_json_object(&root.join("ci").join("config.contract.json"));
    let suite_manifest_path = contract
        .get("ci")
        .and_then(JsonValue::as_object)
        .and_then(|ci| ci.get("suite_manifest_path"))
        .and_then(JsonValue::as_str)
        .and_then(|value| normalize_text(Some(value)))?;
    let catalog_path = root.join(suite_manifest_path);
    catalog_path.is_file().then_some(catalog_path)
}

fn workflow_land_patchset_command(
    task_id: &str,
    base_line_name: &str,
    worktree_retarget: Option<&JsonValue>,
) -> String {
    let publish_command = format!("ait patchset publish {task_id} --summary \"review summary\"");
    let Some(worktree_retarget) = worktree_retarget.and_then(JsonValue::as_object) else {
        return publish_command;
    };
    if normalize_json_text(worktree_retarget.get("rebase_state")).as_deref() == Some("conflicted") {
        return "ait worktree rebase --continue".to_string();
    }
    if worktree_retarget
        .get("needs_retarget")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return format!("ait worktree rebase --onto {base_line_name}");
    }
    publish_command
}

pub fn workflow_ready_command_hints(
    ctx: &dyn WorkflowContext,
    _change_id: &str,
    task_id: &str,
    patchset: Option<&JsonValue>,
    base_line_name: &str,
    worktree_retarget: Option<&JsonValue>,
) -> WorkflowContextResult<JsonValue> {
    let config = merged_config(ctx)?;
    let root = ctx_path(ctx, "root")?;
    let patchset_id = patchset
        .and_then(JsonValue::as_object)
        .and_then(|patchset| normalize_json_text(patchset.get("patchset_id")))
        .map(|value| crate::public_references::public_patchset_reference(&value));
    let publish_command =
        workflow_land_patchset_command(task_id, base_line_name, worktree_retarget);
    let patchset_ci_command =
        if let Some(patchset_id) = patchset_id.as_ref().filter(|_| root.is_none()) {
            JsonValue::String(format!("ait patchset rerun-ci {patchset_id}"))
        } else if let (Some(root), Some(patchset_id)) = (root.as_ref(), patchset_id.as_ref()) {
            if local_patchset_ci_contract_exists(root) {
                JsonValue::String(format!("ait patchset rerun-ci {patchset_id}"))
            } else {
                JsonValue::Null
            }
        } else {
            JsonValue::Null
        };
    let attest_command = patchset_id
        .as_ref()
        .map(|patchset_id| JsonValue::String(format!("ait attest put {patchset_id} --tests pass")))
        .unwrap_or(JsonValue::Null);
    let attestation_command = if root.is_none() {
        attest_command.clone()
    } else if !patchset_ci_command.is_null() {
        patchset_ci_command.clone()
    } else {
        attest_command.clone()
    };
    let apply_command = format!("ait workflow ready {task_id} --apply");
    let code_review_summary_command = patchset_id
        .as_ref()
        .map(|patchset_id| {
            JsonValue::String(format!(
                "ait review code submit {task_id} --patchset {patchset_id} --message \"{CODE_REVIEW_SUMMARY_TEMPLATE}\""
            ))
        })
        .unwrap_or(JsonValue::Null);
    let task_review_required = task_review_enabled(ctx)?;
    let auto_review_reviewer = if task_review_required {
        None
    } else {
        task_review_reviewer_identity(&config)
    };
    let manual_review_command = if task_review_required {
        patchset_id
            .as_ref()
            .map(|patchset_id| {
                JsonValue::String(format!(
                    "ait review task approve {task_id} --patchset {patchset_id} --message \"<functional validation>\""
                ))
            })
            .unwrap_or(JsonValue::Null)
    } else {
        JsonValue::Null
    };
    let review_command = if task_review_required {
        manual_review_command.clone()
    } else if auto_review_reviewer.is_some() {
        JsonValue::String(apply_command.clone())
    } else {
        JsonValue::String("ait config set --user-name \"<name>\"".to_string())
    };
    Ok(json!({
        "apply_command": apply_command,
        "publish_command": publish_command,
        "patchset_ci_command": patchset_ci_command,
        "attest_command": attest_command,
        "attestation_command": attestation_command,
        "code_review_summary_command": code_review_summary_command,
        "code_review_template_command": if patchset_id.is_some() {
            JsonValue::String(CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND.to_string())
        } else {
            JsonValue::Null
        },
        "review_command": review_command,
        "manual_review_command": manual_review_command,
        "auto_review_reviewer": auto_review_reviewer,
        "land_command": format!("ait task finish {task_id}"),
    }))
}

#[allow(clippy::too_many_arguments)]
pub fn workflow_land_command_hints(
    ctx: &dyn WorkflowContext,
    _change_id: &str,
    task_id: &str,
    patchset: Option<&JsonValue>,
    base_line_name: &str,
    _target_line: &str,
    worktree_retarget: Option<&JsonValue>,
    review_blocking: i64,
    requires_code_review_summary: bool,
    task_review_required: bool,
) -> WorkflowContextResult<JsonValue> {
    let config = merged_config(ctx)?;
    let root = ctx_path(ctx, "root")?;
    let team_review_enabled = team_review_enabled(&config);
    let patchset_id = patchset
        .and_then(JsonValue::as_object)
        .and_then(|patchset| normalize_json_text(patchset.get("patchset_id")))
        .map(|value| crate::public_references::public_patchset_reference(&value));
    let apply_command = format!("ait task finish {task_id}");
    let publish_command =
        workflow_land_patchset_command(task_id, base_line_name, worktree_retarget);
    let patchset_ci_command =
        if let Some(patchset_id) = patchset_id.as_ref().filter(|_| root.is_none()) {
            JsonValue::String(format!("ait patchset rerun-ci {patchset_id}"))
        } else if let (Some(root), Some(patchset_id)) = (root.as_ref(), patchset_id.as_ref()) {
            if local_patchset_ci_contract_exists(root) {
                JsonValue::String(format!("ait patchset rerun-ci {patchset_id}"))
            } else {
                JsonValue::Null
            }
        } else {
            JsonValue::Null
        };
    let attest_command = patchset_id
        .as_ref()
        .map(|patchset_id| JsonValue::String(format!("ait attest put {patchset_id} --tests pass")))
        .unwrap_or(JsonValue::Null);
    let attestation_command = if root.is_none() {
        attest_command.clone()
    } else if !patchset_ci_command.is_null() {
        patchset_ci_command.clone()
    } else {
        attest_command.clone()
    };
    let code_review_summary_command = patchset_id.as_ref().map(|patchset_id| {
        JsonValue::String(format!(
            "ait review code submit {task_id} --patchset {patchset_id} --message \"{CODE_REVIEW_SUMMARY_TEMPLATE}\""
        ))
    }).unwrap_or(JsonValue::Null);
    let auto_review_reviewer = if task_review_required {
        None
    } else {
        task_review_reviewer_identity(&config)
    };
    let manual_review_command = if task_review_required {
        patchset_id
            .as_ref()
            .map(|patchset_id| {
                JsonValue::String(format!(
                    "ait review task approve {task_id} --patchset {patchset_id} --message \"<functional validation>\""
                ))
            })
            .unwrap_or(JsonValue::Null)
    } else {
        JsonValue::Null
    };
    let team_review_command = if let Some(patchset_id) = patchset_id.as_ref() {
        if team_review_enabled {
            JsonValue::String(format!(
                "ait review team approve {task_id} --patchset {patchset_id}"
            ))
        } else {
            JsonValue::Null
        }
    } else if team_review_enabled {
        JsonValue::String(format!("ait review team approve {task_id}"))
    } else {
        JsonValue::Null
    };
    let ready_command = format!("ait workflow ready {task_id} --apply");
    let review_command = if review_blocking > 0 {
        JsonValue::String(format!("ait review show {task_id}"))
    } else if task_review_required {
        manual_review_command.clone()
    } else if auto_review_reviewer.is_some() {
        JsonValue::String(ready_command.clone())
    } else {
        JsonValue::String("ait config set --user-name \"<name>\"".to_string())
    };
    let land_command = apply_command.clone();
    Ok(json!({
        "publish_command": publish_command,
        "apply_command": apply_command,
        "ready_command": ready_command,
        "patchset_ci_command": patchset_ci_command,
        "attest_command": attest_command,
        "attestation_command": attestation_command,
        "code_review_summary_command": code_review_summary_command,
        "code_review_template_command": if patchset_id.is_some() && requires_code_review_summary {
            JsonValue::String(CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND.to_string())
        } else {
            JsonValue::Null
        },
        "review_command": review_command,
        "manual_review_command": manual_review_command,
        "team_review_command": team_review_command,
        "auto_review_reviewer": auto_review_reviewer,
        "policy_command": patchset_id.as_ref().map(|patchset_id| JsonValue::String(format!("ait policy eval {patchset_id}"))).unwrap_or(JsonValue::Null),
        "land_command": land_command,
        "task_land_command": format!("ait task finish {task_id}"),
    }))
}

fn parse_iso_datetime(value: Option<&JsonValue>) -> Option<DateTime<FixedOffset>> {
    if let Some(seconds) = value.and_then(JsonValue::as_i64) {
        return DateTime::<Utc>::from_timestamp(seconds, 0).map(|value| value.fixed_offset());
    }
    let text = normalize_json_text(value)?;
    DateTime::parse_from_rfc3339(&text.replace('Z', "+00:00")).ok()
}

fn bounded_poll_seconds(value: f64) -> i64 {
    value.round().clamp(
        WORKFLOW_WAIT_HINT_MIN_SECONDS as f64,
        WORKFLOW_WAIT_HINT_MAX_SECONDS as f64,
    ) as i64
}

fn coerce_wait_hint_seconds(value: Option<&JsonValue>) -> Option<i64> {
    let numeric = value.and_then(JsonValue::as_f64)?;
    if numeric <= 0.0 {
        return None;
    }
    Some(bounded_poll_seconds(numeric))
}

fn duration_seconds(start_value: Option<&JsonValue>, end_value: Option<&JsonValue>) -> Option<i64> {
    let start = parse_iso_datetime(start_value)?;
    let end = parse_iso_datetime(end_value)?;
    let seconds = (end - start).num_seconds();
    if seconds <= 0 {
        return None;
    }
    Some(bounded_poll_seconds(seconds as f64))
}

fn wait_hint_cache_key(kind: &str) -> WorkflowContextResult<&'static str> {
    match kind {
        "ready" => Ok(WORKFLOW_READY_POLL_SECONDS_KEY),
        "land" => Ok(WORKFLOW_LAND_POLL_SECONDS_KEY),
        _ => Err(WorkflowContextError::invalid(format!(
            "Unsupported workflow wait-hint kind: {kind}"
        ))),
    }
}

fn wait_hint_persistence_supported(ctx: &dyn WorkflowContext) -> WorkflowContextResult<bool> {
    Ok(ctx_path(ctx, "config_path")?.is_some() && ctx_path(ctx, "workspace_dir")?.is_some())
}

fn load_cached_wait_hint_seconds(
    ctx: &dyn WorkflowContext,
    kind: &str,
) -> WorkflowContextResult<(Option<i64>, bool)> {
    if !wait_hint_persistence_supported(ctx)? {
        return Ok((None, false));
    }
    let config = merged_config(ctx)?;
    let key = wait_hint_cache_key(kind)?;
    if !config.contains_key(key) {
        return Ok((None, false));
    }
    Ok((coerce_wait_hint_seconds(config.get(key)), true))
}

fn write_wait_hint_seconds(
    ctx: &dyn WorkflowContext,
    kind: &str,
    seconds: Option<i64>,
    mark_bootstrap_attempt: bool,
) -> WorkflowContextResult<Option<i64>> {
    let key = wait_hint_cache_key(kind)?;
    let normalized = seconds.map(|seconds| bounded_poll_seconds(seconds as f64));
    if !wait_hint_persistence_supported(ctx)? {
        return Ok(normalized);
    }
    if let Some(seconds) = normalized {
        update_base_config_value(ctx, key, JsonValue::Number(seconds.into()))?;
        return Ok(Some(seconds));
    }
    if mark_bootstrap_attempt {
        update_base_config_value(
            ctx,
            key,
            JsonValue::Number(WORKFLOW_WAIT_HINT_BOOTSTRAP_MISS.into()),
        )?;
    } else {
        remove_base_config_value(ctx, key)?;
    }
    Ok(None)
}

fn history_wait_hint_sample(detail: &JsonValue, kind: &str) -> WorkflowContextResult<Option<i64>> {
    let patchset = detail
        .get("selected_patchset")
        .and_then(JsonValue::as_object)
        .or_else(|| {
            detail
                .get("current_patchset")
                .and_then(JsonValue::as_object)
        });
    let patchset_ci_status = detail
        .get("patchset_ci_status")
        .and_then(JsonValue::as_object);
    let change = detail.get("change").and_then(JsonValue::as_object);
    match kind {
        "ready" => Ok(duration_seconds(
            patchset.and_then(|value| value.get("created_at")),
            patchset_ci_status.and_then(|value| value.get("ci_completed_at_s")),
        )),
        "land" => Ok(duration_seconds(
            patchset_ci_status.and_then(|value| value.get("ci_completed_at_s")),
            change.and_then(|value| value.get("landed_at")),
        )),
        _ => Err(WorkflowContextError::invalid(format!(
            "Unsupported workflow wait-hint kind: {kind}"
        ))),
    }
}

fn http_auth_headers(config: &Map<String, JsonValue>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    let actor = normalize_text(env::var(names::AIT_NATIVE_ACTOR).ok().as_deref()).or_else(|| {
        normalize_json_text(config.get("user_email"))
            .or_else(|| normalize_json_text(config.get("user_name")))
    });
    if let Some(actor) = actor {
        headers.insert("X-AIT-Actor".to_string(), actor);
    }
    headers
}

fn bootstrap_wait_hint_seconds_from_history(
    ctx: &dyn WorkflowContext,
    kind: &str,
    remote_name: Option<&str>,
) -> WorkflowContextResult<Option<i64>> {
    if !wait_hint_persistence_supported(ctx)? {
        return Ok(None);
    }
    let (remote, repo_name) = remote_tuple(ctx, remote_name)?;
    let base_url = normalize_json_text(remote.get("url"))
        .ok_or_else(|| WorkflowContextError::invalid("Remote row is missing url."))?;
    let config = merged_config(ctx)?;
    let manager_config = PlanHttpClientConfig {
        base_url,
        headers: http_auth_headers(&config),
        ..PlanHttpClientConfig::default()
    };
    let mut manager = PlanHttpClientManager::new(manager_config)
        .map_err(|err| WorkflowContextError::invalid(err.to_string()))?;
    // Wait hints are advisory. Remote timeouts or transient read failures should
    // not block the closeout preview/read-model path.
    let change_rows = match manager.list_changes(&repo_name) {
        Ok(rows) => rows,
        Err(_) => return Ok(None),
    };
    let mut candidates: Vec<(DateTime<FixedOffset>, String)> = Vec::new();
    for row in &change_rows {
        if normalize_json_text(row.get("status")).as_deref() != Some("landed") {
            continue;
        }
        let current_patchset_number = row
            .get("current_patchset_number")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let selected_patchset_number = row
            .get("selected_patchset_number")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        if current_patchset_number != 1 || selected_patchset_number != 1 {
            continue;
        }
        let landed_at = match parse_iso_datetime(row.get("landed_at")) {
            Some(landed_at) => landed_at,
            None => continue,
        };
        let Some(change_id) = normalize_json_text(row.get("change_id")) else {
            continue;
        };
        candidates.push((landed_at, change_id));
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    candidates.sort_by_key(|candidate| candidate.0);
    let mut samples = Vec::new();
    for (_landed_at, change_id) in candidates
        .iter()
        .rev()
        .take(WORKFLOW_WAIT_HINT_HISTORY_LIMIT)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let detail = match manager.get_change_detail(change_id, Some(&repo_name)) {
            Ok(detail) => detail,
            Err(_) => continue,
        };
        if let Some(sample) = history_wait_hint_sample(&detail, kind)? {
            samples.push(sample);
        }
    }
    if samples.is_empty() {
        return Ok(None);
    }
    let start_index = samples
        .len()
        .saturating_sub(WORKFLOW_WAIT_HINT_SAMPLE_LIMIT);
    let window = &samples[start_index..];
    let mut ema = window[0] as f64;
    for sample in &window[1..] {
        ema =
            (ema * (1.0 - WORKFLOW_WAIT_HINT_ALPHA)) + (*sample as f64 * WORKFLOW_WAIT_HINT_ALPHA);
    }
    Ok(Some(bounded_poll_seconds(ema)))
}

fn resolve_wait_hint_seconds(
    ctx: &dyn WorkflowContext,
    kind: &str,
    state: &JsonValue,
) -> WorkflowContextResult<Option<i64>> {
    let (cached, cache_present) = load_cached_wait_hint_seconds(ctx, kind)?;
    if cached.is_some() {
        return Ok(cached);
    }
    if cache_present {
        return Ok(None);
    }
    let remote_name = state
        .get("resolved_remote_name")
        .and_then(JsonValue::as_str);
    let seeded = bootstrap_wait_hint_seconds_from_history(ctx, kind, remote_name)?;
    write_wait_hint_seconds(ctx, kind, seeded, true)
}

pub fn workflow_closeout_wait_hint(
    ctx: &dyn WorkflowContext,
    state: &JsonValue,
    next_action: &JsonValue,
) -> WorkflowContextResult<Option<JsonValue>> {
    let code = normalize_json_text(next_action.get("code"));
    if ctx_path(ctx, "root")?.is_none() {
        return match code.as_deref() {
            Some("waiting_for_ci") => {
                let mut payload = Map::new();
                payload.insert(
                    "kind".to_string(),
                    JsonValue::String("patchset_ci".to_string()),
                );
                payload.insert("poll_seconds".to_string(), JsonValue::Number(42.into()));
                if let Some(status_ref) = state
                    .get("patchset")
                    .and_then(JsonValue::as_object)
                    .and_then(|patchset| normalize_json_text(patchset.get("patchset_id")))
                {
                    payload.insert("status_ref".to_string(), JsonValue::String(status_ref));
                }
                Ok(Some(JsonValue::Object(payload)))
            }
            Some("waiting_for_land") => {
                let mut payload = Map::new();
                payload.insert(
                    "kind".to_string(),
                    JsonValue::String("land_submission".to_string()),
                );
                payload.insert("poll_seconds".to_string(), JsonValue::Number(88.into()));
                let status_ref = state
                    .get("landing_summary")
                    .and_then(JsonValue::as_object)
                    .and_then(|landing_summary| {
                        normalize_json_text(landing_summary.get("submission_id"))
                    })
                    .or_else(|| normalize_json_text(state.get("landing_submission_id")));
                if let Some(status_ref) = status_ref {
                    payload.insert("status_ref".to_string(), JsonValue::String(status_ref));
                }
                Ok(Some(JsonValue::Object(payload)))
            }
            _ => Ok(None),
        };
    }
    match code.as_deref() {
        Some("waiting_for_ci") => {
            let Some(seconds) = resolve_wait_hint_seconds(ctx, "ready", state)? else {
                return Ok(None);
            };
            let mut payload = Map::new();
            payload.insert(
                "kind".to_string(),
                JsonValue::String("patchset_ci".to_string()),
            );
            payload.insert(
                "poll_seconds".to_string(),
                JsonValue::Number(seconds.into()),
            );
            if let Some(status_ref) = state
                .get("patchset")
                .and_then(JsonValue::as_object)
                .and_then(|patchset| normalize_json_text(patchset.get("patchset_id")))
            {
                payload.insert("status_ref".to_string(), JsonValue::String(status_ref));
            }
            Ok(Some(JsonValue::Object(payload)))
        }
        Some("waiting_for_land") => {
            let Some(seconds) = resolve_wait_hint_seconds(ctx, "land", state)? else {
                return Ok(None);
            };
            let mut payload = Map::new();
            payload.insert(
                "kind".to_string(),
                JsonValue::String("land_submission".to_string()),
            );
            payload.insert(
                "poll_seconds".to_string(),
                JsonValue::Number(seconds.into()),
            );
            let status_ref = state
                .get("landing_summary")
                .and_then(JsonValue::as_object)
                .and_then(|landing_summary| {
                    normalize_json_text(landing_summary.get("submission_id"))
                })
                .or_else(|| normalize_json_text(state.get("landing_submission_id")));
            if let Some(status_ref) = status_ref {
                payload.insert("status_ref".to_string(), JsonValue::String(status_ref));
            }
            Ok(Some(JsonValue::Object(payload)))
        }
        _ => Ok(None),
    }
}

use crate::workflow_closeout_facts::{
    workflow_land_full_facts as build_rust_workflow_land_full_facts,
    workflow_landed_facts as build_rust_workflow_landed_facts,
    workflow_ready_facts as build_rust_workflow_ready_facts,
};
use crate::workflow_closeout_read_model::{
    project_workflow_land_full_read_model as project_rust_workflow_land_full_read_model,
    project_workflow_landed_read_model as project_rust_workflow_landed_read_model,
    project_workflow_ready_read_model as project_rust_workflow_ready_read_model,
};

pub fn project_workflow_landed_read_model_for_context(
    ctx: &dyn WorkflowContext,
    state: &JsonValue,
) -> WorkflowContextResult<JsonValue> {
    let facts = build_rust_workflow_landed_facts(state).map_err(WorkflowContextError::invalid)?;
    let facts_state = facts.as_object().cloned().unwrap_or_default();
    let command_hints = workflow_land_command_hints(
        ctx,
        string_or_empty(&facts_state, "change", "change_id").as_str(),
        string_or_empty(&facts_state, "task", "task_id").as_str(),
        facts_state.get("patchset"),
        string_or_empty_root(&facts_state, "target_line").as_str(),
        string_or_empty_root(&facts_state, "target_line").as_str(),
        None,
        0,
        false,
        true,
    )?;
    let payload = project_rust_workflow_landed_read_model(&facts, &command_hints)
        .map_err(WorkflowContextError::invalid)?;
    Ok(payload)
}

pub fn project_workflow_land_full_read_model_for_context(
    ctx: &dyn WorkflowContext,
    state: &JsonValue,
    apply_owned_continuation: bool,
) -> WorkflowContextResult<JsonValue> {
    let landed_facts =
        build_rust_workflow_landed_facts(state).map_err(WorkflowContextError::invalid)?;
    if landed_facts
        .as_object()
        .and_then(|value| value.get("landed"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return project_workflow_landed_read_model_for_context(ctx, state);
    }
    let facts =
        build_rust_workflow_land_full_facts(state).map_err(WorkflowContextError::invalid)?;
    let facts_state = facts.as_object().cloned().unwrap_or_default();
    let task_review_enabled = task_review_enabled(ctx)?;
    let command_hints = workflow_land_command_hints(
        ctx,
        string_or_empty_root(&facts_state, "resolved_change_id").as_str(),
        string_or_empty(&facts_state, "task", "task_id").as_str(),
        facts_state.get("patchset"),
        string_or_empty_root(&facts_state, "base_line_name").as_str(),
        string_or_empty_root(&facts_state, "target_line").as_str(),
        facts_state.get("worktree_retarget"),
        int_or_zero_root(&facts_state, "review_blocking"),
        bool_or_false_root(&facts_state, "requires_code_review_summary"),
        task_review_enabled,
    )?;
    let task_review_config = effective_task_review(ctx)?;
    let mut payload = project_rust_workflow_land_full_read_model(
        &facts,
        &command_hints,
        &task_review_config,
        apply_owned_continuation,
    )
    .map_err(WorkflowContextError::invalid)?;
    let next_action = payload
        .as_object()
        .and_then(|value| value.get("next_action"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    if let Some(wait_hint) = workflow_closeout_wait_hint(ctx, state, &next_action)? {
        if let JsonValue::Object(ref mut object) = payload {
            object.insert("wait_hint".to_string(), wait_hint);
        }
    }
    Ok(payload)
}

pub fn project_workflow_ready_read_model_for_context(
    ctx: &dyn WorkflowContext,
    state: &JsonValue,
    ignore_workspace_authoring: bool,
    patchset_is_authoritative: bool,
    apply_owned_continuation: bool,
) -> WorkflowContextResult<JsonValue> {
    let facts = build_rust_workflow_ready_facts(state).map_err(WorkflowContextError::invalid)?;
    let facts_state = facts.as_object().cloned().unwrap_or_default();
    let command_hints = workflow_ready_command_hints(
        ctx,
        string_or_empty(&facts_state, "change", "change_id").as_str(),
        string_or_empty(&facts_state, "task", "task_id").as_str(),
        facts_state.get("patchset"),
        string_or_empty_root(&facts_state, "base_line_name").as_str(),
        facts_state.get("worktree_retarget"),
    )?;
    let mut payload = project_rust_workflow_ready_read_model(
        &facts,
        &command_hints,
        ignore_workspace_authoring,
        patchset_is_authoritative,
        apply_owned_continuation,
    )
    .map_err(WorkflowContextError::invalid)?;
    let next_action = payload
        .as_object()
        .and_then(|value| value.get("next_action"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    if let Some(wait_hint) = workflow_closeout_wait_hint(ctx, state, &next_action)? {
        if let JsonValue::Object(ref mut object) = payload {
            object.insert("wait_hint".to_string(), wait_hint);
        }
    }
    Ok(payload)
}

fn string_or_empty(root: &Map<String, JsonValue>, object_key: &str, field_key: &str) -> String {
    root.get(object_key)
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get(field_key))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_default()
}

fn string_or_empty_root(root: &Map<String, JsonValue>, field_key: &str) -> String {
    root.get(field_key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_default()
}

fn int_or_zero_root(root: &Map<String, JsonValue>, field_key: &str) -> i64 {
    root.get(field_key).and_then(JsonValue::as_i64).unwrap_or(0)
}

fn bool_or_false_root(root: &Map<String, JsonValue>, field_key: &str) -> bool {
    root.get(field_key)
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_context_overlay_and_wait_cache_remain_native() {
        let fixture = tempfile::tempdir().unwrap();
        let base = fixture.path().join("config.json");
        let overlay = fixture.path().join("worktree.json");
        fs::write(
            &base,
            "{\"task_review\":false,\"workflow_ready_poll_seconds\":30}",
        )
        .unwrap();
        fs::write(
            &overlay,
            "{\"task_review\":true,\"workflow_ready_poll_seconds\":55}",
        )
        .unwrap();
        let context = WorkflowContextData {
            root: Some(fixture.path().into()),
            config_path: Some(base.clone()),
            worktree_config_path: Some(overlay),
            workspace_dir: Some(fixture.path().join("work")),
            ..Default::default()
        };
        assert_eq!(
            effective_task_review(&context).unwrap(),
            json!({"value":true,"source":"repo_config"})
        );
        let before = fs::read(&base).unwrap();
        let hint = workflow_closeout_wait_hint(
            &context,
            &json!({"patchset":{"patchset_id":"RT-1/P-01"}}),
            &json!({"code":"waiting_for_ci"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            hint,
            json!({"kind":"patchset_ci","poll_seconds":55,"status_ref":"RT-1/P-01"})
        );
        assert_eq!(fs::read(&base).unwrap(), before);
        write_wait_hint_seconds(&context, "land", Some(1000), false).unwrap();
        assert_eq!(read_json_object(&base)["workflow_land_poll_seconds"], 900);
        let explicit = WorkflowContextData {
            task_review: Some(false),
            ..context
        };
        assert_eq!(
            effective_task_review(&explicit).unwrap(),
            json!({"value":false})
        );
    }

    #[test]
    fn retained_context_without_root_preserves_existing_wait_hints() {
        let context = WorkflowContextData::default();
        assert_eq!(
            workflow_closeout_wait_hint(&context, &json!({}), &json!({"code":"waiting_for_ci"}))
                .unwrap()
                .unwrap()["poll_seconds"],
            42
        );
        assert_eq!(
            workflow_closeout_wait_hint(&context, &json!({}), &json!({"code":"waiting_for_land"}))
                .unwrap()
                .unwrap()["poll_seconds"],
            88
        );
        assert!(matches!(
            ctx_required_path(&context, "config_path"),
            Err(WorkflowContextError::Invalid(_))
        ));
    }
}
