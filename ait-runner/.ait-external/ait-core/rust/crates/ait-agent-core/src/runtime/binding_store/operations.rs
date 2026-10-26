use ait_core::json_support::{json, JsonMap, JsonNumber, JsonValue};
use ait_core::runtime_binding_state::DEFAULT_RUNTIME_BINDING_STATE_VERSION;
use chrono::{SecondsFormat, Utc};

use super::AgentRuntimeBindingStore;

const RETIRED_SESSION_FIELDS: &[&str] = &[
    "session_id",
    "canonical_session_id",
    "branch_session_id",
    "active_session_id",
    "previous_session_id",
    "shared_session_canonical_session_id",
    "shared_session_branch_session_id",
    "binding_role",
    "last_relink_skipped_from_session_id",
    "last_sync_at",
];

impl AgentRuntimeBindingStore {
    pub fn execute(&self, operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
        match operation {
            "load" => self.load(),
            "save" => {
                let payload = request_field(request, "state")
                    .or_else(|| request_field(request, "payload"))
                    .ok_or_else(|| "runtime binding save operation requires state".to_string())?;
                self.save(payload)
            }
            "recover_interrupted_writes" => Ok(json!(self.recover_interrupted_writes()?)),
            "update_last_update_id" => self.mutate(|state| {
                let update_id = request_i64(request, "update_id")?.max(0);
                let current = state_object(state)?
                    .get("last_update_id")
                    .and_then(JsonValue::as_i64)
                    .unwrap_or(0);
                if update_id <= current {
                    return Ok((state.clone(), false));
                }
                state_object_mut(state)?.insert(
                    "last_update_id".to_string(),
                    JsonValue::Number(JsonNumber::from(update_id)),
                );
                Ok((state.clone(), true))
            }),
            "get_binding" => {
                let state = self.load()?;
                Ok(get_binding(&state, request).unwrap_or(JsonValue::Null))
            }
            "list_bindings" => {
                let state = self.load()?;
                Ok(JsonValue::Array(list_bindings(&state, request)?))
            }
            "upsert_binding" => self.mutate(|state| {
                let binding = upsert_binding_state(state, request)?;
                Ok((binding, true))
            }),
            "patch_binding" => self.mutate(|state| match patch_binding_state(state, request)? {
                Some(binding) => Ok((binding, true)),
                None => Ok((JsonValue::Null, false)),
            }),
            "get_bootstrap_auth" => {
                let state = self.load()?;
                Ok(state_object(&state)?
                    .get("telegram_bootstrap_auth")
                    .cloned()
                    .unwrap_or_else(|| json!({})))
            }
            "save_bootstrap_auth" => self.mutate(|state| {
                let payload = request_field(request, "payload")
                    .or_else(|| request_field(request, "bootstrap_auth"))
                    .and_then(JsonValue::as_object)
                    .cloned()
                    .ok_or_else(|| {
                        "runtime binding save_bootstrap_auth requires object payload".to_string()
                    })?;
                state_object_mut(state)?.insert(
                    "telegram_bootstrap_auth".to_string(),
                    JsonValue::Object(payload.clone()),
                );
                Ok((JsonValue::Object(payload), true))
            }),
            "has_recent_value" => {
                let state = self.load()?;
                Ok(JsonValue::Bool(has_recent_value(&state, request)?))
            }
            "remember_recent_value" => {
                self.mutate(|state| match remember_recent_value_state(state, request)? {
                    Some(binding) => Ok((binding, true)),
                    None => Ok((JsonValue::Null, false)),
                })
            }
            other => Err(format!(
                "unsupported ait-agent runtime binding store operation '{other}'"
            )),
        }
    }

    pub(crate) fn mutate_binding_with<F>(
        &self,
        transport: &str,
        surface_id: &JsonValue,
        mutation: F,
    ) -> Result<Option<JsonValue>, String>
    where
        F: FnOnce(Option<&JsonValue>) -> Result<Option<JsonValue>, String>,
    {
        let lookup = json!({"transport": transport, "surface_id": surface_id});
        let result = self.mutate(|state| {
            let current = get_binding(state, &lookup);
            let Some(updates) = mutation(current.as_ref())? else {
                return Ok((current.unwrap_or(JsonValue::Null), false));
            };
            if !updates.is_object() {
                return Err("runtime binding mutation updates must be an object".to_string());
            }
            let request = json!({
                "transport": transport,
                "surface_id": surface_id,
                "updates": updates,
            });
            match patch_binding_state(state, &request)? {
                Some(binding) => Ok((binding, true)),
                None => Ok((JsonValue::Null, false)),
            }
        })?;
        match result {
            JsonValue::Null => Ok(None),
            JsonValue::Object(_) => Ok(Some(result)),
            _ => Err("runtime binding mutation returned an invalid binding".to_string()),
        }
    }
}

fn get_binding(state: &JsonValue, request: &JsonValue) -> Option<JsonValue> {
    let key = binding_lookup_key(request).ok()?;
    state_field_object(state, "surface_bindings")
        .get(&key)
        .filter(|value| value.is_object())
        .cloned()
}

fn list_bindings(state: &JsonValue, request: &JsonValue) -> Result<Vec<JsonValue>, String> {
    let request = request_object(request)?;
    let repo_name = clean_text(request.get("repo_name"));
    let transport = clean_text(request.get("transport"));
    let include_inactive = request
        .get("include_inactive")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    Ok(state_field_object(state, "surface_bindings")
        .values()
        .filter_map(JsonValue::as_object)
        .filter(|binding| {
            repo_name
                .as_ref()
                .is_none_or(|wanted| clean_text(binding.get("repo_name")).as_ref() == Some(wanted))
        })
        .filter(|binding| {
            transport
                .as_ref()
                .is_none_or(|wanted| clean_text(binding.get("transport")).as_ref() == Some(wanted))
        })
        .filter(|binding| {
            include_inactive
                || clean_text(binding.get("status")).unwrap_or_else(|| "active".to_string())
                    == "active"
        })
        .cloned()
        .map(JsonValue::Object)
        .collect())
}

fn upsert_binding_state(state: &mut JsonValue, request: &JsonValue) -> Result<JsonValue, String> {
    let request = request_object(request)?;
    let transport = required_text(request, "transport")?;
    let surface_id = required_text(request, "surface_id")?;
    let thread_id = clean_text(request.get("thread_id"));
    let key = surface_binding_id(&transport, &surface_id, thread_id.as_deref());
    let mut bindings = state_field_object(state, "surface_bindings");
    let current = bindings
        .get(&key)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let now = clean_text(request.get("now_iso")).unwrap_or_else(now_iso);
    let mut binding = current.clone();
    binding.insert("binding_id".to_string(), JsonValue::String(key.clone()));
    binding.insert("transport".to_string(), JsonValue::String(transport));
    binding.insert("surface_id".to_string(), JsonValue::String(surface_id));
    set_optional_text(&mut binding, "thread_id", thread_id);
    for field in ["surface_title", "surface_kind", "repo_name", "status"] {
        if request.contains_key(field) {
            set_optional_value(&mut binding, field, request.get(field));
        }
    }
    if !binding.contains_key("status") {
        binding.insert(
            "status".to_string(),
            JsonValue::String("active".to_string()),
        );
    }
    if !binding.contains_key("linked_at") {
        binding.insert("linked_at".to_string(), JsonValue::String(now.clone()));
    }
    apply_updates(
        &mut binding,
        request.get("updates").and_then(JsonValue::as_object),
    );
    binding.insert("updated_at".to_string(), JsonValue::String(now));
    strip_retired_session_fields(&mut binding);
    bindings.insert(key, JsonValue::Object(binding.clone()));
    let state = state_object_mut(state)?;
    state.insert("surface_bindings".to_string(), JsonValue::Object(bindings));
    state.insert(
        "version".to_string(),
        JsonValue::Number(JsonNumber::from(DEFAULT_RUNTIME_BINDING_STATE_VERSION)),
    );
    Ok(JsonValue::Object(binding))
}

fn patch_binding_state(
    state: &mut JsonValue,
    request: &JsonValue,
) -> Result<Option<JsonValue>, String> {
    let key = binding_lookup_key(request)?;
    let mut bindings = state_field_object(state, "surface_bindings");
    let Some(mut binding) = bindings.get(&key).and_then(JsonValue::as_object).cloned() else {
        return Ok(None);
    };
    let request = request_object(request)?;
    apply_updates(
        &mut binding,
        request.get("updates").and_then(JsonValue::as_object),
    );
    binding.insert(
        "updated_at".to_string(),
        JsonValue::String(clean_text(request.get("now_iso")).unwrap_or_else(now_iso)),
    );
    strip_retired_session_fields(&mut binding);
    bindings.insert(key, JsonValue::Object(binding.clone()));
    state_object_mut(state)?.insert("surface_bindings".to_string(), JsonValue::Object(bindings));
    Ok(Some(JsonValue::Object(binding)))
}

fn has_recent_value(state: &JsonValue, request: &JsonValue) -> Result<bool, String> {
    let request_object = request_object(request)?;
    let Some(value) = clean_text(request_object.get("value")) else {
        return Ok(false);
    };
    let recent_key = required_text(request_object, "recent_key")?;
    let Some(binding) = get_binding(state, request).and_then(|value| value.as_object().cloned())
    else {
        return Ok(false);
    };
    Ok(binding
        .get(&recent_key)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| clean_text(Some(entry)))
        .any(|entry| entry == value))
}

fn remember_recent_value_state(
    state: &mut JsonValue,
    request: &JsonValue,
) -> Result<Option<JsonValue>, String> {
    let request_object = request_object(request)?;
    let key = binding_lookup_key(request)?;
    let Some(current) = state_field_object(state, "surface_bindings")
        .get(&key)
        .and_then(JsonValue::as_object)
        .cloned()
    else {
        return Ok(None);
    };
    let value = required_text(request_object, "value")?;
    let recent_key = required_text(request_object, "recent_key")?;
    let last_value_key = required_text(request_object, "last_value_key")?;
    let mut recent = current
        .get(&recent_key)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| clean_text(Some(entry)))
        .filter(|entry| entry != &value)
        .collect::<Vec<_>>();
    recent.push(value.clone());
    let limit = request_object
        .get("limit")
        .map(coerce_i64)
        .transpose()?
        .unwrap_or(64)
        .max(1) as usize;
    if recent.len() > limit {
        recent = recent.split_off(recent.len() - limit);
    }
    let mut updates = request_object
        .get("updates")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    updates.insert(
        recent_key,
        JsonValue::Array(recent.into_iter().map(JsonValue::String).collect()),
    );
    updates.insert(last_value_key, JsonValue::String(value));
    if let Some(sequence) = request_object.get("last_synced_sequence") {
        updates.insert(
            "last_synced_sequence".to_string(),
            JsonValue::Number(JsonNumber::from(coerce_i64(sequence)?.max(0))),
        );
    }
    let mut patch = request_object.clone();
    patch.insert("updates".to_string(), JsonValue::Object(updates));
    patch_binding_state(state, &JsonValue::Object(patch))
}

pub fn agent_runtime_binding_projection_json(binding: &JsonValue) -> Result<JsonValue, String> {
    let binding = binding
        .as_object()
        .ok_or_else(|| "ait-agent runtime binding projection requires an object".to_string())?;
    let surface_id = clean_text(binding.get("surface_id"));
    Ok(json!({
        "binding_id": clean_text(binding.get("binding_id")),
        "transport": clean_text(binding.get("transport")),
        "surface_id": surface_id,
        "thread_id": clean_text(binding.get("thread_id")),
        "conversation_key": clean_text(binding.get("conversation_key")),
        "provider_thread": binding.get("codex_thread_binding").cloned().unwrap_or(JsonValue::Null),
        "surface_label": clean_text(binding.get("surface_title")).or(surface_id),
    }))
}

fn binding_lookup_key(request: &JsonValue) -> Result<String, String> {
    let request = request_object(request)?;
    if let Some(binding_id) = clean_text(request.get("binding_id")) {
        return Ok(binding_id);
    }
    Ok(surface_binding_id(
        clean_text(request.get("transport"))
            .as_deref()
            .unwrap_or("unknown"),
        clean_text(request.get("surface_id"))
            .as_deref()
            .unwrap_or("unknown"),
        clean_text(request.get("thread_id")).as_deref(),
    ))
}

fn surface_binding_id(transport: &str, surface_id: &str, thread_id: Option<&str>) -> String {
    match thread_id {
        Some(thread_id) => format!("{transport}:{surface_id}:thread:{thread_id}"),
        None => format!("{transport}:{surface_id}"),
    }
}

fn apply_updates(
    binding: &mut JsonMap<String, JsonValue>,
    updates: Option<&JsonMap<String, JsonValue>>,
) {
    let Some(updates) = updates else { return };
    for (key, value) in updates {
        if RETIRED_SESSION_FIELDS.contains(&key.as_str()) {
            continue;
        }
        if value.is_null() {
            binding.remove(key);
        } else {
            binding.insert(key.clone(), value.clone());
        }
    }
}

fn strip_retired_session_fields(binding: &mut JsonMap<String, JsonValue>) {
    for field in RETIRED_SESSION_FIELDS {
        binding.remove(*field);
    }
}

fn set_optional_text(binding: &mut JsonMap<String, JsonValue>, key: &str, value: Option<String>) {
    match value {
        Some(value) => {
            binding.insert(key.to_string(), JsonValue::String(value));
        }
        None => {
            binding.remove(key);
        }
    }
}

fn set_optional_value(
    binding: &mut JsonMap<String, JsonValue>,
    key: &str,
    value: Option<&JsonValue>,
) {
    match value {
        Some(value) if !value.is_null() => {
            binding.insert(key.to_string(), value.clone());
        }
        _ => {
            binding.remove(key);
        }
    }
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = match value {
        Some(JsonValue::String(value)) => value.trim().to_string(),
        Some(JsonValue::Number(value)) => value.to_string(),
        Some(JsonValue::Bool(value)) => value.to_string(),
        _ => String::new(),
    };
    (!value.is_empty()).then_some(value)
}

fn request_object(request: &JsonValue) -> Result<&JsonMap<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "runtime binding store operation request must be an object".to_string())
}

fn request_field<'a>(request: &'a JsonValue, key: &str) -> Option<&'a JsonValue> {
    request.as_object()?.get(key)
}

fn required_text(request: &JsonMap<String, JsonValue>, key: &str) -> Result<String, String> {
    clean_text(request.get(key))
        .ok_or_else(|| format!("runtime binding store operation requires {key}"))
}

fn request_i64(request: &JsonValue, key: &str) -> Result<i64, String> {
    let value = request_field(request, key)
        .ok_or_else(|| format!("runtime binding store operation requires {key}"))?;
    coerce_i64(value)
}

fn coerce_i64(value: &JsonValue) -> Result<i64, String> {
    match value {
        JsonValue::Number(value) => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .ok_or_else(|| "runtime binding integer is out of range".to_string()),
        JsonValue::String(value) => value
            .trim()
            .parse::<i64>()
            .map_err(|_| "runtime binding integer is malformed".to_string()),
        _ => Err("runtime binding integer is malformed".to_string()),
    }
}

fn state_object(state: &JsonValue) -> Result<&JsonMap<String, JsonValue>, String> {
    state
        .as_object()
        .ok_or_else(|| "normalized runtime binding state must be an object".to_string())
}

fn state_object_mut(state: &mut JsonValue) -> Result<&mut JsonMap<String, JsonValue>, String> {
    state
        .as_object_mut()
        .ok_or_else(|| "normalized runtime binding state must be an object".to_string())
}

fn state_field_object(state: &JsonValue, key: &str) -> JsonMap<String, JsonValue> {
    state
        .as_object()
        .and_then(|state| state.get(key))
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default()
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Micros, false)
}
