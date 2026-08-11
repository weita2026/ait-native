use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::json_support::{encode_json_value_pretty, parse_json_object_or_empty};
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use ait_core::plan_http_client::{PlanHttpClientConfig, PlanHttpClientManager};
use ait_core::remote_store::{remote_by_name_with_remote_store, ConfigRemoteStore};
use chrono::{DateTime, FixedOffset, Utc};
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;

const DEFAULT_TASK_REVIEW: bool = false;
const DEFAULT_WORKFLOW_SCOPE: &str = "local";
const DEFAULT_PLAN_TASK_BINDING_MODE: &str = "required";
const WORKFLOW_READY_POLL_SECONDS_KEY: &str = "workflow_ready_poll_seconds";
const WORKFLOW_LAND_POLL_SECONDS_KEY: &str = "workflow_land_poll_seconds";
const WORKFLOW_WAIT_HINT_BOOTSTRAP_MISS: i64 = 0;
const WORKFLOW_WAIT_HINT_ALPHA: f64 = 0.5;
const WORKFLOW_WAIT_HINT_MIN_SECONDS: i64 = 5;
const WORKFLOW_WAIT_HINT_MAX_SECONDS: i64 = 900;
const WORKFLOW_WAIT_HINT_HISTORY_LIMIT: usize = 40;
const WORKFLOW_WAIT_HINT_SAMPLE_LIMIT: usize = 12;
const CODE_REVIEW_SUMMARY_TEMPLATE: &str =
    "Reviewed files: <paths reviewed>; Findings: <blocking/non-blocking findings>; Risks: <residual risks>; Tests: <checks run>; Recommendation: <land/defer/request changes>";
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

fn ctx_text(ctx: &Bound<'_, PyAny>, attr_name: &str) -> PyResult<Option<String>> {
    let value = match ctx.getattr(attr_name) {
        Ok(value) => value,
        Err(err) if err.is_instance_of::<PyKeyError>(ctx.py()) => return Ok(None),
        Err(err) if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(ctx.py()) => {
            return Ok(None)
        }
        Err(err) => return Err(err),
    };
    if value.is_none() {
        return Ok(None);
    }
    Ok(normalize_text(Some(
        value.str()?.to_string_lossy().as_ref(),
    )))
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

fn ctx_bool_toggle(ctx: &Bound<'_, PyAny>, attr_name: &str) -> PyResult<Option<bool>> {
    let value = match ctx.getattr(attr_name) {
        Ok(value) => value,
        Err(err) if err.is_instance_of::<PyKeyError>(ctx.py()) => return Ok(None),
        Err(err) if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(ctx.py()) => {
            return Ok(None)
        }
        Err(err) => return Err(err),
    };
    if value.is_none() {
        return Ok(None);
    }
    if let Ok(flag) = value.extract::<bool>() {
        return Ok(Some(flag));
    }
    let text = value.str()?.to_string_lossy().to_string();
    Ok(match text.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    })
}

fn ctx_path(ctx: &Bound<'_, PyAny>, attr_name: &str) -> PyResult<Option<PathBuf>> {
    let value = match ctx.getattr(attr_name) {
        Ok(value) => value,
        Err(err) if err.is_instance_of::<PyKeyError>(ctx.py()) => return Ok(None),
        Err(err) if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(ctx.py()) => {
            return Ok(None)
        }
        Err(err) => return Err(err),
    };
    if value.is_none() {
        return Ok(None);
    }
    let text = value.str()?.to_string_lossy().trim().to_string();
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(text)))
}

fn ctx_required_path(ctx: &Bound<'_, PyAny>, attr_name: &str) -> PyResult<PathBuf> {
    ctx_path(ctx, attr_name)?.ok_or_else(|| {
        PyValueError::new_err(format!("Task/workflow context must expose {attr_name}."))
    })
}

fn read_json_object(path: &Path) -> Map<String, JsonValue> {
    let Ok(text) = fs::read_to_string(path) else {
        return Map::new();
    };
    parse_json_object_or_empty(&text)
}

fn merged_config(ctx: &Bound<'_, PyAny>) -> PyResult<Map<String, JsonValue>> {
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

fn base_config_path(ctx: &Bound<'_, PyAny>) -> PyResult<PathBuf> {
    ctx_required_path(ctx, "config_path")
}

fn update_base_config_value(ctx: &Bound<'_, PyAny>, key: &str, value: JsonValue) -> PyResult<()> {
    let config_path = base_config_path(ctx)?;
    let mut base = read_json_object(&config_path);
    base.insert(key.to_string(), value);
    let payload =
        encode_json_value_pretty(&JsonValue::Object(base)).map_err(PyValueError::new_err)?;
    fs::write(config_path, payload).map_err(|err| PyValueError::new_err(err.to_string()))?;
    Ok(())
}

fn remove_base_config_value(ctx: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
    let config_path = base_config_path(ctx)?;
    let mut base = read_json_object(&config_path);
    base.remove(key);
    let payload =
        encode_json_value_pretty(&JsonValue::Object(base)).map_err(PyValueError::new_err)?;
    fs::write(config_path, payload).map_err(|err| PyValueError::new_err(err.to_string()))?;
    Ok(())
}

pub fn effective_task_review(ctx: &Bound<'_, PyAny>) -> PyResult<JsonValue> {
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

pub fn task_review_enabled(ctx: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(effective_task_review(ctx)?
        .get("value")
        .and_then(JsonValue::as_bool)
        .unwrap_or(DEFAULT_TASK_REVIEW))
}

fn plan_task_binding_mode(value: Option<&JsonValue>) -> Option<String> {
    match value {
        Some(JsonValue::Object(obj)) => {
            normalize_json_text(obj.get("mode")).and_then(|mode| match mode.as_str() {
                "advisory" | "strict" | "required" => Some(mode),
                _ => None,
            })
        }
        _ => None,
    }
}

fn effective_workflow_mode(config: &Map<String, JsonValue>) -> String {
    let configured_mode = normalize_json_text(config.get("workflow_mode"));
    let workflow_scope = normalize_json_text(config.get("workflow_default_scope"))
        .unwrap_or_else(|| DEFAULT_WORKFLOW_SCOPE.to_string());
    let task_scope = normalize_json_text(config.get("task_default_scope"))
        .unwrap_or_else(|| workflow_scope.clone());
    let change_scope = normalize_json_text(config.get("change_default_scope"))
        .unwrap_or_else(|| workflow_scope.clone());
    let binding_mode = plan_task_binding_mode(config.get("plan_task_binding"))
        .unwrap_or_else(|| DEFAULT_PLAN_TASK_BINDING_MODE.to_string());
    if let Some(mode) = configured_mode.as_deref() {
        let preset = match mode {
            "solo_local" => Some(("local", "local", "local", "required")),
            "solo_remote" => Some(("remote", "remote", "remote", "required")),
            "team_remote" => Some(("remote", "remote", "remote", "required")),
            _ => None,
        };
        if let Some((preset_workflow, preset_task, preset_change, preset_binding)) = preset {
            if workflow_scope == preset_workflow
                && task_scope == preset_task
                && change_scope == preset_change
                && binding_mode == preset_binding
            {
                return mode.to_string();
            }
        }
    }
    if workflow_scope == "local"
        && task_scope == "local"
        && change_scope == "local"
        && binding_mode == "required"
    {
        return "solo_local".to_string();
    }
    if workflow_scope == "remote"
        && task_scope == "remote"
        && change_scope == "remote"
        && binding_mode == "advisory"
    {
        return "solo_remote".to_string();
    }
    if workflow_scope == "remote"
        && task_scope == "remote"
        && change_scope == "remote"
        && binding_mode == "required"
    {
        return "team_remote".to_string();
    }
    "custom".to_string()
}

fn team_review_enabled(config: &Map<String, JsonValue>) -> bool {
    effective_workflow_mode(config) == "team_remote"
}

fn lookup_remote_row(config_path: &Path, name: &str) -> PyResult<Map<String, JsonValue>> {
    let store = ConfigRemoteStore::new(config_path).map_err(PyValueError::new_err)?;
    let remote = remote_by_name_with_remote_store(&store, name)
        .map_err(PyValueError::new_err)?
        .ok_or_else(|| PyKeyError::new_err(format!("Unknown remote: {name}")))?;
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
    ctx: &Bound<'_, PyAny>,
    remote_name: Option<&str>,
) -> PyResult<(Map<String, JsonValue>, String)> {
    let config = merged_config(ctx)?;
    let resolved_remote_name = normalize_text(remote_name).or_else(|| {
        config
            .get("default_remote")
            .and_then(JsonValue::as_str)
            .map(str::to_string)
    });
    let Some(resolved_remote_name) = resolved_remote_name else {
        return Err(PyKeyError::new_err(
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
        .ok_or_else(|| PyKeyError::new_err("Remote context is missing repo_name."))?;
    Ok((remote, repo_name))
}

fn task_review_auto_approval_reviewer_identity(config: &Map<String, JsonValue>) -> Option<String> {
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
    change_id: &str,
    base_line_name: &str,
    worktree_retarget: Option<&JsonValue>,
) -> String {
    let publish_command =
        format!("ait patchset publish --change {change_id} --summary \"review summary\"");
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
    ctx: &Bound<'_, PyAny>,
    change_id: &str,
    patchset: Option<&JsonValue>,
    base_line_name: &str,
    worktree_retarget: Option<&JsonValue>,
) -> PyResult<JsonValue> {
    let root = ctx_path(ctx, "root")?;
    let patchset_id = patchset
        .and_then(JsonValue::as_object)
        .and_then(|patchset| normalize_json_text(patchset.get("patchset_id")));
    let publish_command =
        workflow_land_patchset_command(change_id, base_line_name, worktree_retarget);
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
    Ok(json!({
        "apply_command": format!("ait workflow ready {change_id} --apply"),
        "publish_command": publish_command,
        "patchset_ci_command": patchset_ci_command,
        "attest_command": attest_command,
        "attestation_command": attestation_command,
        "land_command": format!("ait task land {change_id}"),
    }))
}

#[allow(clippy::too_many_arguments)]
pub fn workflow_land_command_hints(
    ctx: &Bound<'_, PyAny>,
    change_id: &str,
    _task_id: &str,
    patchset: Option<&JsonValue>,
    base_line_name: &str,
    _target_line: &str,
    worktree_retarget: Option<&JsonValue>,
    review_blocking: i64,
    requires_code_review_summary: bool,
    task_review_enabled: bool,
) -> PyResult<JsonValue> {
    let config = merged_config(ctx)?;
    let root = ctx_path(ctx, "root")?;
    let team_review_enabled = team_review_enabled(&config);
    let patchset_id = patchset
        .and_then(JsonValue::as_object)
        .and_then(|patchset| normalize_json_text(patchset.get("patchset_id")));
    let apply_command = format!("ait task land {change_id}");
    let publish_command =
        workflow_land_patchset_command(change_id, base_line_name, worktree_retarget);
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
            "ait review code submit {change_id} --patchset {patchset_id} --verdict pass --message \"{CODE_REVIEW_SUMMARY_TEMPLATE}\""
        ))
    }).unwrap_or(JsonValue::Null);
    let auto_review_reviewer = if task_review_enabled {
        None
    } else {
        task_review_auto_approval_reviewer_identity(&config)
            .or_else(|| ctx_text(ctx, "user_name").ok().flatten())
    };
    let manual_review_command = if let Some(patchset_id) = patchset_id.as_ref() {
        format!("ait review task approve {change_id} --patchset {patchset_id}")
    } else {
        format!("ait review task approve {change_id}")
    };
    let team_review_command = if let Some(patchset_id) = patchset_id.as_ref() {
        if team_review_enabled {
            JsonValue::String(format!(
                "ait review team approve {change_id} --patchset {patchset_id}"
            ))
        } else {
            JsonValue::Null
        }
    } else if team_review_enabled {
        JsonValue::String(format!("ait review team approve {change_id}"))
    } else {
        JsonValue::Null
    };
    let review_command = if review_blocking > 0 {
        format!("ait review show {change_id}")
    } else if auto_review_reviewer.is_some() {
        apply_command.clone()
    } else {
        manual_review_command.clone()
    };
    let land_command = apply_command.clone();
    Ok(json!({
        "publish_command": publish_command,
        "apply_command": apply_command,
        "ready_command": format!("ait workflow ready {change_id} --apply"),
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
        "task_complete_command": format!("ait task land {change_id}"),
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

fn wait_hint_cache_key(kind: &str) -> PyResult<&'static str> {
    match kind {
        "ready" => Ok(WORKFLOW_READY_POLL_SECONDS_KEY),
        "land" => Ok(WORKFLOW_LAND_POLL_SECONDS_KEY),
        _ => Err(PyValueError::new_err(format!(
            "Unsupported workflow wait-hint kind: {kind}"
        ))),
    }
}

fn wait_hint_persistence_supported(ctx: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(ctx_path(ctx, "config_path")?.is_some() && ctx_path(ctx, "workspace_dir")?.is_some())
}

fn load_cached_wait_hint_seconds(
    ctx: &Bound<'_, PyAny>,
    kind: &str,
) -> PyResult<(Option<i64>, bool)> {
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
    ctx: &Bound<'_, PyAny>,
    kind: &str,
    seconds: Option<i64>,
    mark_bootstrap_attempt: bool,
) -> PyResult<Option<i64>> {
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

fn history_wait_hint_sample(detail: &JsonValue, kind: &str) -> PyResult<Option<i64>> {
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
        _ => Err(PyValueError::new_err(format!(
            "Unsupported workflow wait-hint kind: {kind}"
        ))),
    }
}

fn http_auth_headers(config: &Map<String, JsonValue>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    let actor = normalize_text(env::var("AIT_NATIVE_ACTOR").ok().as_deref())
        .or_else(|| normalize_text(env::var("AIT_ACTOR").ok().as_deref()))
        .or_else(|| {
            normalize_json_text(config.get("user_email"))
                .or_else(|| normalize_json_text(config.get("user_name")))
        });
    let actor_type = normalize_text(env::var("AIT_NATIVE_ACTOR_TYPE").ok().as_deref())
        .or_else(|| normalize_text(env::var("AIT_ACTOR_TYPE").ok().as_deref()));
    let roles = normalize_text(env::var("AIT_NATIVE_ROLES").ok().as_deref())
        .or_else(|| normalize_text(env::var("AIT_ROLES").ok().as_deref()));
    let repos = normalize_text(env::var("AIT_NATIVE_REPOS").ok().as_deref())
        .or_else(|| normalize_text(env::var("AIT_REPOS").ok().as_deref()));
    if let Some(actor) = actor {
        headers.insert("X-AIT-Actor".to_string(), actor);
    }
    if let Some(actor_type) = actor_type {
        headers.insert("X-AIT-Actor-Type".to_string(), actor_type);
    }
    if let Some(roles) = roles {
        headers.insert("X-AIT-Roles".to_string(), roles);
    }
    if let Some(repos) = repos {
        headers.insert("X-AIT-Repos".to_string(), repos);
    }
    headers
}

fn bootstrap_wait_hint_seconds_from_history(
    ctx: &Bound<'_, PyAny>,
    kind: &str,
    remote_name: Option<&str>,
) -> PyResult<Option<i64>> {
    if !wait_hint_persistence_supported(ctx)? {
        return Ok(None);
    }
    let (remote, repo_name) = remote_tuple(ctx, remote_name)?;
    let base_url = normalize_json_text(remote.get("url"))
        .ok_or_else(|| PyValueError::new_err("Remote row is missing url."))?;
    let config = merged_config(ctx)?;
    let manager_config = PlanHttpClientConfig {
        base_url,
        headers: http_auth_headers(&config),
        ..PlanHttpClientConfig::default()
    };
    let mut manager = PlanHttpClientManager::new(manager_config)
        .map_err(|err| PyValueError::new_err(err.to_string()))?;
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
    ctx: &Bound<'_, PyAny>,
    kind: &str,
    state: &JsonValue,
) -> PyResult<Option<i64>> {
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
    ctx: &Bound<'_, PyAny>,
    state: &JsonValue,
    next_action: &JsonValue,
) -> PyResult<Option<JsonValue>> {
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
