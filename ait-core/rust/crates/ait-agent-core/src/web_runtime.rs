use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use ait_core::json_support::{json, JsonMap, JsonValue};

use crate::manifest::{
    agent_select_telegram_worker_json, list_manifest_workers, AgentWorkerManifestStore,
};
use crate::runtime::{
    agent_runtime_binding_projection_json, agent_runtime_binding_store_execute_json,
};
use crate::transport::TransportKind;
use crate::transport_config::{
    resolve_agent_worker_config, AgentWorkerConfigInput, AgentWorkerRuntimeConfig,
};

pub const AGENT_WEB_RUNTIME_CONTRACT: &str = "ait.agent.web_runtime.v1";

pub fn agent_web_runtime_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request
        .as_object()
        .ok_or_else(|| "ait-agent web runtime request must be an object".to_string())?;
    let operation = required_text(request.get("operation"), "operation")?;
    let result = match operation.as_str() {
        "binding_project" => binding_project(request)?,
        "binding_store_execute" => binding_store_execute(request)?,
        "telegram_config_load" => telegram_config_load(request)?,
        "telegram_env_path_resolve" => telegram_env_path_resolve(request)?,
        _ => {
            return Err(format!(
                "unsupported ait-agent web runtime operation `{operation}`"
            ))
        }
    };
    Ok(json!({
        "contract": AGENT_WEB_RUNTIME_CONTRACT,
        "operation": operation,
        "result": result,
        "python_policy_execution_allowed": false,
    }))
}

fn binding_project(request: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let binding = request
        .get("binding")
        .ok_or_else(|| "ait-agent web binding projection requires binding".to_string())?;
    agent_runtime_binding_projection_json(binding)
}

fn binding_store_execute(request: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let store_request = request
        .get("request")
        .ok_or_else(|| "ait-agent web binding store execution requires request".to_string())?;
    agent_runtime_binding_store_execute_json(store_request)
}

fn telegram_env_path_resolve(request: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let repo_root = PathBuf::from(required_text(request.get("repo_root"), "repo_root")?);
    let process_env = request_process_env(request)?;
    let default_path = repo_root
        .join(".ait")
        .join("agent-runtime")
        .join("telegram.env");
    let selected = optional_text(request.get("value"))
        .map(|value| select_safe_repo_override(&repo_root, &default_path, &value, &process_env))
        .unwrap_or(default_path);
    Ok(json!({"path": selected.to_string_lossy()}))
}

fn telegram_config_load(request: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let repo_root = PathBuf::from(required_text(request.get("repo_root"), "repo_root")?);
    let process_env = request_process_env(request)?;
    let default_manifest_path = repo_root.join(".ait").join("agent-workers.json");
    let manifest_path = clean_map_text(process_env.get("AIT_AGENT_CONFIG_PATH"))
        .map(|value| {
            select_safe_repo_override(&repo_root, &default_manifest_path, &value, &process_env)
        })
        .unwrap_or(default_manifest_path);
    let document = AgentWorkerManifestStore::filesystem(&manifest_path).load();
    if !document.issues.is_empty() {
        return Err(format!(
            "ait-agent worker manifest is invalid: {}",
            document.issues.join("; ")
        ));
    }
    let requested_name = optional_text(request.get("name"))
        .or_else(|| clean_map_text(process_env.get("AIT_TELEGRAM_GRAPH_TRIGGER_WORKER")))
        .unwrap_or_else(|| "main".to_string());
    let selected = agent_select_telegram_worker_json(&document.config, Some(&requested_name));
    let worker = if selected.as_object().is_some() {
        selected
    } else {
        let telegram_workers = list_manifest_workers(&document.config)
            .into_iter()
            .filter(|worker| worker.transport == TransportKind::Telegram)
            .count();
        if telegram_workers > 0 {
            return Err(format!(
                "Telegram worker `{requested_name}` is not configured and the manifest selection is ambiguous"
            ));
        }
        json!({"kind": "telegram", "name": requested_name})
    };
    let worker_name = worker
        .get("name")
        .and_then(JsonValue::as_str)
        .and_then(clean_text)
        .unwrap_or_else(|| requested_name.clone());
    let worker_key = format!("telegram/{worker_name}");
    let config = resolve_agent_worker_config(AgentWorkerConfigInput {
        repo_root: repo_root.clone(),
        worker_key,
        worker,
        process_env,
    })?;
    let AgentWorkerRuntimeConfig::Telegram(config) = config else {
        return Err("ait-agent web runtime selected a non-Telegram worker".to_string());
    };
    Ok(json!({
        "repo_root": config.shared.runtime_target.repo_root.to_string_lossy(),
        "worker_name": config.shared.worker_name,
        "token": config.token.expose(),
        "username": config.username,
        "repo_name": config.shared.runtime_target.repo_name,
        "request_timeout_seconds": config.shared.request_timeout_seconds,
        "reply_markdown_enabled": config.reply_markdown_enabled,
        "ait_web_url": config.shared.ait_web_url,
        "background_sync_enabled": config.background_sync_enabled,
        "background_sync_interval_seconds": config.background_sync_interval_seconds,
        "env_path": config.shared.paths.env_path,
        "manifest_path": manifest_path.to_string_lossy(),
    }))
}

fn request_process_env(
    request: &JsonMap<String, JsonValue>,
) -> Result<BTreeMap<String, String>, String> {
    let Some(value) = request.get("process_env") else {
        return Ok(env::vars().collect());
    };
    let object = value
        .as_object()
        .ok_or_else(|| "ait-agent web runtime process_env must be an object".to_string())?;
    object
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_string()))
                .ok_or_else(|| {
                    format!("ait-agent web runtime process_env value `{key}` must be a string")
                })
        })
        .collect()
}

fn select_safe_repo_override(
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

fn resolve_path(repo_root: &Path, value: &str, process_env: &BTreeMap<String, String>) -> PathBuf {
    let value = value.trim();
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

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    optional_text(value).ok_or_else(|| format!("ait-agent web runtime requires {field}"))
}

fn optional_text(value: Option<&JsonValue>) -> Option<String> {
    value.and_then(JsonValue::as_str).and_then(clean_text)
}

fn clean_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn clean_map_text(value: Option<&String>) -> Option<String> {
    value.and_then(|value| clean_text(value))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ait_core::json_support::{json, JsonValue};
    use tempfile::TempDir;

    use super::*;

    fn result(response: JsonValue) -> JsonValue {
        assert_eq!(response["contract"], AGENT_WEB_RUNTIME_CONTRACT);
        assert_eq!(response["python_policy_execution_allowed"], false);
        response["result"].clone()
    }

    #[test]
    fn web_runtime_projects_sessionless_binding_in_rust() {
        let projected = result(
            agent_web_runtime_execute_json(&json!({
                "operation": "binding_project",
                "binding": {
                    "binding_id": "telegram:42:thread:topic-1",
                    "transport": "telegram",
                    "surface_id": "42",
                    "thread_id": "topic-1",
                    "conversation_key": "telegram:42:thread:topic-1",
                    "codex_thread_binding": {"thread_id": "codex-thread-1"},
                    "surface_title": "Ops"
                }
            }))
            .expect("project binding"),
        );

        assert_eq!(projected["transport"], "telegram");
        assert_eq!(projected["surface_id"], "42");
        assert_eq!(projected["thread_id"], "topic-1");
        assert_eq!(projected["conversation_key"], "telegram:42:thread:topic-1");
        assert_eq!(projected["provider_thread"]["thread_id"], "codex-thread-1");
        assert_eq!(projected["surface_label"], "Ops");
        assert!(!projected.to_string().contains("session"));
    }

    #[test]
    fn web_runtime_keeps_existing_repo_env_path_over_unsafe_override() {
        let repo = TempDir::new().expect("repo tempdir");
        let outside = TempDir::new().expect("outside tempdir");
        let default_path = repo.path().join(".ait/agent-runtime/telegram.env");
        fs::create_dir_all(default_path.parent().expect("env parent")).expect("create env parent");
        fs::write(&default_path, "BOT_TOKEN=repo\n").expect("write repo env");

        let selected = result(
            agent_web_runtime_execute_json(&json!({
                "operation": "telegram_env_path_resolve",
                "repo_root": repo.path().to_string_lossy(),
                "value": outside.path().join("telegram.env").to_string_lossy(),
                "process_env": {}
            }))
            .expect("resolve env path"),
        );

        assert_eq!(selected["path"], default_path.to_string_lossy().as_ref());
    }

    #[test]
    fn web_runtime_loads_typed_telegram_config_from_manifest() {
        let repo = TempDir::new().expect("repo tempdir");
        fs::create_dir_all(repo.path().join(".ait")).expect("create ait dir");
        fs::write(
            repo.path().join(".ait/agent-workers.json"),
            r#"{"version":1,"workers":{"telegram/main":{"token":"secret","username":"@ait","env_path":".ait/agent-runtime/telegram.env"}}}"#,
        )
        .expect("write manifest");

        let config = result(
            agent_web_runtime_execute_json(&json!({
                "operation": "telegram_config_load",
                "repo_root": repo.path().to_string_lossy(),
                "name": "main",
                "process_env": {
                    "AIT_TELEGRAM_BACKGROUND_SYNC_ENABLED": "true",
                    "AIT_TELEGRAM_BACKGROUND_SYNC_INTERVAL_SECONDS": "12.5"
                }
            }))
            .expect("load Telegram config"),
        );

        assert_eq!(config["worker_name"], "main");
        assert_eq!(config["token"], "secret");
        assert_eq!(config["username"], "ait");
        assert_eq!(config["background_sync_enabled"], true);
        assert_eq!(config["background_sync_interval_seconds"], 12.5);
        assert_eq!(
            config["env_path"],
            repo.path()
                .join(".ait/agent-runtime/telegram.env")
                .to_string_lossy()
                .as_ref()
        );
    }

    #[test]
    fn web_runtime_keeps_existing_repo_manifest_over_unsafe_override() {
        let repo = TempDir::new().expect("repo tempdir");
        let outside = TempDir::new().expect("outside tempdir");
        fs::create_dir_all(repo.path().join(".ait")).expect("create ait dir");
        let default_manifest = repo.path().join(".ait/agent-workers.json");
        fs::write(
            &default_manifest,
            r#"{"version":1,"workers":{"telegram/main":{"token":"repo-secret"}}}"#,
        )
        .expect("write repo manifest");
        let outside_manifest = outside.path().join("workers.json");
        fs::write(
            &outside_manifest,
            r#"{"version":1,"workers":{"telegram/main":{"token":"outside-secret"}}}"#,
        )
        .expect("write outside manifest");

        let config = result(
            agent_web_runtime_execute_json(&json!({
                "operation": "telegram_config_load",
                "repo_root": repo.path().to_string_lossy(),
                "process_env": {
                    "AIT_AGENT_CONFIG_PATH": outside_manifest.to_string_lossy()
                }
            }))
            .expect("load safe Telegram config"),
        );

        assert_eq!(config["token"], "repo-secret");
        assert_eq!(
            config["manifest_path"],
            default_manifest.to_string_lossy().as_ref()
        );
    }

    #[test]
    fn web_runtime_routes_binding_mutation_through_rust_store() {
        let temp = TempDir::new().expect("binding tempdir");
        let state_path = temp.path().join("bindings.json");
        let response = result(
            agent_web_runtime_execute_json(&json!({
                "operation": "binding_store_execute",
                "request": {
                    "path": state_path.to_string_lossy(),
                    "operation": "upsert_binding",
                    "transport": "telegram",
                    "surface_id": "42",
                    "repo_name": "demo",
                    "updates": {
                        "conversation_key": "telegram:42"
                    }
                }
            }))
            .expect("upsert binding"),
        );

        assert_eq!(response["contract"], "ait.agent.runtime_binding_store.v2");
        assert_eq!(response["operation"], "upsert_binding");
        assert_eq!(response["python_file_mutation_allowed"], false);
        assert_eq!(response["result"]["conversation_key"], "telegram:42");
        assert!(!response["result"].to_string().contains("session"));
        assert!(state_path.is_file());
    }

    #[test]
    fn web_runtime_rejects_unknown_operations() {
        let error = agent_web_runtime_execute_json(&json!({"operation": "python_fallback"}))
            .expect_err("unknown operation must fail");

        assert!(error.contains("unsupported ait-agent web runtime operation"));
    }
}
