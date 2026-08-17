use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_supervisor_public_payload_contract";

const SECRET_FIELDS: &[&str] = &["token", "secret", "app_token", "bot_token"];

pub fn agent_supervisor_public_worker_payload_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "supervisor public worker payload request must be an object".to_string())?;
    let worker = object
        .get("worker")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            "supervisor public worker payload request requires worker object".to_string()
        })?;

    let mut payload = public_worker_payload(worker);

    if payload.get("bot_token_set") != Some(&JsonValue::Bool(true)) {
        if let Some(env_bot_token) = clean_text(object.get("env_bot_token")) {
            merge_map(
                &mut payload,
                redact_named_token("bot_token", &env_bot_token),
            );
        }
    }

    if object.contains_key("config") || object.contains_key("config_issues") {
        let config = object.get("config").and_then(JsonValue::as_object);
        let issues = string_array(object.get("config_issues"));
        payload.insert(
            "config_version".to_string(),
            config
                .and_then(|value| value.get("version"))
                .cloned()
                .unwrap_or_else(|| json!(1)),
        );
        payload.insert(
            "config_valid".to_string(),
            JsonValue::Bool(issues.is_empty()),
        );
        if !issues.is_empty() {
            payload.insert(
                "config_issues".to_string(),
                JsonValue::Array(issues.into_iter().map(JsonValue::String).collect()),
            );
        }
    }

    if let Some(status) = object.get("process_status").and_then(JsonValue::as_object) {
        if let Some(paths) = object.get("paths").and_then(JsonValue::as_object) {
            merge_process_status(&mut payload, status, paths);
        }
    }

    payload.insert(
        "python_worker_execution_allowed".to_string(),
        JsonValue::Bool(false),
    );
    payload.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    Ok(JsonValue::Object(payload))
}

fn public_worker_payload(worker: &Map<String, JsonValue>) -> Map<String, JsonValue> {
    let mut payload = Map::new();
    for (key, value) in worker {
        if !SECRET_FIELDS.contains(&key.as_str()) {
            payload.insert(key.clone(), value.clone());
        }
    }
    merge_map(
        &mut payload,
        redact_named_token(
            "token",
            &clean_text(worker.get("token")).unwrap_or_default(),
        ),
    );
    merge_map(
        &mut payload,
        redact_named_token(
            "secret",
            &clean_text(worker.get("secret")).unwrap_or_default(),
        ),
    );
    merge_map(
        &mut payload,
        redact_named_token(
            "app_token",
            &clean_text(worker.get("app_token")).unwrap_or_default(),
        ),
    );
    merge_map(
        &mut payload,
        redact_named_token(
            "bot_token",
            &clean_text(worker.get("bot_token")).unwrap_or_default(),
        ),
    );
    payload
}

fn merge_process_status(
    payload: &mut Map<String, JsonValue>,
    status: &Map<String, JsonValue>,
    paths: &Map<String, JsonValue>,
) {
    let pid = status
        .get("pid")
        .and_then(JsonValue::as_i64)
        .filter(|pid| *pid > 0);
    let running = pid.is_some() && bool_field(status, "running");
    payload.insert("running".to_string(), JsonValue::Bool(running));
    payload.insert(
        "pid".to_string(),
        pid.map(JsonValue::from).unwrap_or(JsonValue::Null),
    );
    for key in [
        "sync_state_path",
        "pid_file",
        "log_file",
        "env_path",
        "termination_context_path",
    ] {
        payload.insert(
            key.to_string(),
            clean_text(paths.get(key))
                .map(JsonValue::String)
                .unwrap_or_else(|| JsonValue::String(String::new())),
        );
    }
    payload.insert(
        "health".to_string(),
        status
            .get("health")
            .and_then(JsonValue::as_object)
            .cloned()
            .map(JsonValue::Object)
            .unwrap_or_else(|| JsonValue::Object(Map::new())),
    );
}

fn redact_named_token(name: &str, token: &str) -> Map<String, JsonValue> {
    let mut payload = Map::new();
    let raw = token.to_string();
    if raw.is_empty() {
        payload.insert(format!("{name}_set"), JsonValue::Bool(false));
        payload.insert(format!("{name}_preview"), JsonValue::Null);
        return payload;
    }
    let chars: Vec<char> = raw.chars().collect();
    let preview = if chars.len() <= 4 {
        "*".repeat(chars.len())
    } else {
        format!(
            "{}{}",
            "*".repeat((chars.len() - 4).max(4)),
            chars[chars.len() - 4..].iter().collect::<String>()
        )
    };
    payload.insert(format!("{name}_set"), JsonValue::Bool(true));
    payload.insert(format!("{name}_preview"), JsonValue::String(preview));
    payload
}

fn merge_map(target: &mut Map<String, JsonValue>, source: Map<String, JsonValue>) {
    for (key, value) in source {
        target.insert(key, value);
    }
}

fn bool_field(object: &Map<String, JsonValue>, key: &str) -> bool {
    object
        .get(key)
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = match value? {
        JsonValue::String(text) => text.clone(),
        JsonValue::Null => return None,
        other => other.to_string(),
    };
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn string_array(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| clean_text(Some(item)))
                .collect()
        })
        .unwrap_or_default()
}
