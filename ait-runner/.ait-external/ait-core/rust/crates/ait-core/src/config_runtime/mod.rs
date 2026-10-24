use crate::json_support::{JsonMap, JsonValue};

use crate::json_support::JsonCodec;
use crate::shared_foundation::ConfigProvider;

pub struct RuntimeConfigJson<S> {
    store: S,
}

impl<S> RuntimeConfigJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl RuntimeConfigJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> RuntimeConfigJson<S> {
    pub fn normalize_runtime_selection_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload =
            self.parse_object_payload(payload_json, "plan config/runtime selection request")?;
        normalize_plan_runtime_selection_request_payload_map(payload)
    }

    pub fn build_runtime_selection_facts_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload =
            self.parse_object_payload(payload_json, "plan config/runtime selection request")?;
        build_plan_runtime_selection_facts_map(payload)
    }

    pub fn normalize_runtime_selection_facts_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload =
            self.parse_object_payload(payload_json, "plan config/runtime selection facts")?;
        normalize_plan_runtime_selection_facts_payload_map(payload)
    }

    pub fn normalize_runtime_compatibility_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload =
            self.parse_object_payload(payload_json, "plan config/runtime compatibility")?;
        normalize_plan_runtime_compatibility_payload_map(payload)
    }

    pub fn normalize_runtime_readiness_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan config/runtime readiness")?;
        normalize_plan_runtime_readiness_payload_map(payload)
    }

    pub fn normalize_runtime_doctor_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan config/runtime doctor")?;
        normalize_plan_runtime_doctor_payload_map(payload)
    }

    fn parse_object_payload(
        &self,
        payload_json: &str,
        label: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let _ = &self.store;
        JsonCodec::parse_object_with_error_prefix(
            payload_json,
            &format!("Could not decode {label} JSON"),
            &format!("{label} must be an object."),
        )
        .map_err(String::from)
    }
}

#[derive(Default)]
pub struct RuntimeConfigFoundation;

impl ConfigProvider for RuntimeConfigFoundation {
    fn normalize_runtime_selection_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        RuntimeConfigJson::stateless()
            .normalize_runtime_selection_request_payload_json(payload_json)
    }

    fn build_runtime_selection_facts_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        RuntimeConfigJson::stateless().build_runtime_selection_facts_json(payload_json)
    }

    fn normalize_runtime_selection_facts_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        RuntimeConfigJson::stateless().normalize_runtime_selection_facts_payload_json(payload_json)
    }

    fn normalize_runtime_compatibility_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        RuntimeConfigJson::stateless().normalize_runtime_compatibility_payload_json(payload_json)
    }

    fn normalize_runtime_readiness_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        RuntimeConfigJson::stateless().normalize_runtime_readiness_payload_json(payload_json)
    }

    fn normalize_runtime_doctor_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        RuntimeConfigJson::stateless().normalize_runtime_doctor_payload_json(payload_json)
    }
}

pub fn normalize_plan_runtime_selection_request_with_config_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: ConfigProvider + ?Sized,
{
    provider.normalize_runtime_selection_request_payload_json(payload_json)
}

pub fn build_plan_runtime_selection_facts_with_config_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: ConfigProvider + ?Sized,
{
    provider.build_runtime_selection_facts_json(payload_json)
}

pub fn normalize_plan_runtime_selection_facts_with_config_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: ConfigProvider + ?Sized,
{
    provider.normalize_runtime_selection_facts_payload_json(payload_json)
}

pub fn normalize_plan_runtime_compatibility_with_config_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: ConfigProvider + ?Sized,
{
    provider.normalize_runtime_compatibility_payload_json(payload_json)
}

pub fn normalize_plan_runtime_readiness_with_config_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: ConfigProvider + ?Sized,
{
    provider.normalize_runtime_readiness_payload_json(payload_json)
}

pub fn normalize_plan_runtime_doctor_with_config_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: ConfigProvider + ?Sized,
{
    provider.normalize_runtime_doctor_payload_json(payload_json)
}

const PLAN_CONFIG_RUNTIME_SELECTION_KEYS: &[&str] = &[
    "plan_core_backend",
    "plan_http_backend",
    "plan_filesystem_backend",
    "plan_blob_diff_backend",
    "plan_pack_substrate_backend",
    "workflow_primitives_backend",
    "plan_ports_protocols_backend",
    "plan_config_runtime_backend",
];

pub fn normalize_plan_runtime_selection_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    RuntimeConfigJson::stateless().normalize_runtime_selection_request_payload_json(payload_json)
}

fn normalize_plan_runtime_selection_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let overrides_value = payload.get("overrides");
    let overrides = if let Some(value) = overrides_value {
        let overrides_object =
            require_object(Some(value), "plan config/runtime selection overrides")?;
        normalize_selection_overrides(overrides_object)?
    } else {
        JsonMap::new()
    };
    Ok(JsonValue::Object(JsonMap::from_iter([(
        "overrides".to_string(),
        JsonValue::Object(overrides),
    )])))
}

pub fn build_plan_runtime_selection_facts_json(payload_json: &str) -> Result<JsonValue, String> {
    RuntimeConfigJson::stateless().build_runtime_selection_facts_json(payload_json)
}

fn build_plan_runtime_selection_facts_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let request = normalize_plan_runtime_selection_request_payload_map(payload)?;
    let request_object = require_object(Some(&request), "plan config/runtime selection request")?;
    let overrides = require_object(
        request_object.get("overrides"),
        "plan config/runtime selection request overrides",
    )?;
    let mut normalized = JsonMap::new();
    for key in PLAN_CONFIG_RUNTIME_SELECTION_KEYS {
        let fact = selector_fact(key, overrides)?;
        normalized.insert((*key).to_string(), JsonValue::Object(fact));
    }
    Ok(JsonValue::Object(normalized))
}

pub fn normalize_plan_runtime_selection_facts_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    RuntimeConfigJson::stateless().normalize_runtime_selection_facts_payload_json(payload_json)
}

fn normalize_plan_runtime_selection_facts_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    Ok(JsonValue::Object(normalize_selection_facts_map(&payload)?))
}

pub fn normalize_plan_runtime_compatibility_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    RuntimeConfigJson::stateless().normalize_runtime_compatibility_payload_json(payload_json)
}

fn normalize_plan_runtime_compatibility_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let selectors = normalize_selection_facts_map(require_object(
        payload.get("selectors"),
        "plan config/runtime compatibility selectors",
    )?)?;
    let plan_authority = normalize_plan_authority_map(require_object(
        payload.get("plan_authority"),
        "plan config/runtime compatibility plan_authority",
    )?)?;
    let compatible = require_bool(payload.get("compatible"), "compatible")?;
    let issues = normalize_string_list(payload.get("issues"), "issues")?;
    let surface_commands = normalize_command_surface(payload.get("surface_commands"))?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        ("selectors".to_string(), JsonValue::Object(selectors)),
        (
            "plan_authority".to_string(),
            JsonValue::Object(plan_authority),
        ),
        ("compatible".to_string(), JsonValue::Bool(compatible)),
        ("issues".to_string(), JsonValue::Array(issues)),
        (
            "surface_commands".to_string(),
            JsonValue::Array(surface_commands),
        ),
    ])))
}

pub fn normalize_plan_runtime_readiness_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    RuntimeConfigJson::stateless().normalize_runtime_readiness_payload_json(payload_json)
}

fn normalize_plan_runtime_readiness_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let selectors = normalize_selection_facts_map(require_object(
        payload.get("selectors"),
        "plan config/runtime readiness selectors",
    )?)?;
    let plan_authority = normalize_plan_authority_map(require_object(
        payload.get("plan_authority"),
        "plan config/runtime readiness plan_authority",
    )?)?;
    let storage_readiness = match payload.get("storage_readiness") {
        None | Some(JsonValue::Null) => JsonValue::Null,
        Some(_) => {
            return Err(
                "Plan config/runtime storage readiness must be null in this runtime.".to_string(),
            )
        }
    };
    let ready = require_bool(payload.get("ready"), "ready")?;
    let issues = normalize_string_list(payload.get("issues"), "issues")?;
    let surface_commands = normalize_command_surface(payload.get("surface_commands"))?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        ("selectors".to_string(), JsonValue::Object(selectors)),
        (
            "plan_authority".to_string(),
            JsonValue::Object(plan_authority),
        ),
        ("storage_readiness".to_string(), storage_readiness),
        ("ready".to_string(), JsonValue::Bool(ready)),
        ("issues".to_string(), JsonValue::Array(issues)),
        (
            "surface_commands".to_string(),
            JsonValue::Array(surface_commands),
        ),
    ])))
}

pub fn normalize_plan_runtime_doctor_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    RuntimeConfigJson::stateless().normalize_runtime_doctor_payload_json(payload_json)
}

fn normalize_plan_runtime_doctor_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let selectors = normalize_selection_facts_map(require_object(
        payload.get("selectors"),
        "plan config/runtime doctor selectors",
    )?)?;
    let compatibility = match normalize_plan_runtime_compatibility_payload_map(
        require_object(
            payload.get("compatibility"),
            "plan config/runtime doctor compatibility",
        )?
        .clone(),
    )? {
        JsonValue::Object(map) => map,
        _ => {
            return Err(
                "Plan config/runtime doctor compatibility normalization must return an object."
                    .to_string(),
            )
        }
    };
    let readiness = match normalize_plan_runtime_readiness_payload_map(
        require_object(
            payload.get("readiness"),
            "plan config/runtime doctor readiness",
        )?
        .clone(),
    )? {
        JsonValue::Object(map) => map,
        _ => {
            return Err(
                "Plan config/runtime doctor readiness normalization must return an object."
                    .to_string(),
            )
        }
    };
    let env_map = normalize_env_snapshot_map(require_object(
        payload.get("env"),
        "plan config/runtime doctor env",
    )?)?;
    let surface_commands = normalize_command_surface(payload.get("surface_commands"))?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        ("selectors".to_string(), JsonValue::Object(selectors)),
        (
            "compatibility".to_string(),
            JsonValue::Object(compatibility),
        ),
        ("readiness".to_string(), JsonValue::Object(readiness)),
        ("env".to_string(), JsonValue::Object(env_map)),
        (
            "surface_commands".to_string(),
            JsonValue::Array(surface_commands),
        ),
    ])))
}

fn normalize_selection_overrides(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut normalized = JsonMap::new();
    for (key, value) in payload {
        if !PLAN_CONFIG_RUNTIME_SELECTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "Unsupported plan config/runtime selector override `{key}`."
            ));
        }
        if value.is_null() {
            continue;
        }
        let text = require_string(value, &format!("override `{key}`"))?;
        normalized.insert(key.clone(), JsonValue::String(text.trim().to_string()));
    }
    Ok(normalized)
}

fn normalize_selection_facts_map(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut normalized = JsonMap::new();
    for key in PLAN_CONFIG_RUNTIME_SELECTION_KEYS {
        let entry = require_object(
            payload.get(*key),
            &format!("plan config/runtime selection facts entry `{key}`"),
        )?;
        let value = normalize_backend_name(Some(
            require_string(
                entry.get("value").ok_or_else(|| {
                    format!("Plan config/runtime selection facts entry `{key}` is missing value.")
                })?,
                &format!("{key}.value"),
            )?
            .as_str(),
        ))?;
        let source = require_string(
            entry.get("source").ok_or_else(|| {
                format!("Plan config/runtime selection facts entry `{key}` is missing source.")
            })?,
            &format!("{key}.source"),
        )?;
        if !matches!(source.as_str(), "default" | "env" | "explicit") {
            return Err(format!(
                "Unsupported backend selection source `{source}` for `{key}`."
            ));
        }
        normalized.insert(
            (*key).to_string(),
            JsonValue::Object(JsonMap::from_iter([
                ("value".to_string(), JsonValue::String(value)),
                ("source".to_string(), JsonValue::String(source)),
            ])),
        );
    }
    Ok(normalized)
}

fn normalize_plan_authority_map(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut normalized = payload.clone();
    let selected_backend = normalize_backend_name(Some(
        require_string(
            payload.get("selected_backend").ok_or_else(|| {
                "Plan config/runtime plan_authority is missing selected_backend.".to_string()
            })?,
            "plan_authority.selected_backend",
        )?
        .as_str(),
    ))?;
    let selected_backend_ready = require_bool(
        payload.get("selected_backend_ready"),
        "plan_authority.selected_backend_ready",
    )?;
    let rust_authority_ready = require_bool(
        payload.get("rust_authority_ready"),
        "plan_authority.rust_authority_ready",
    )?;
    let compatibility =
        require_nonempty_text(payload.get("compatibility"), "plan_authority.compatibility")?;
    let issues = normalize_string_list(payload.get("issues"), "plan_authority.issues")?;
    normalized.insert(
        "selected_backend".to_string(),
        JsonValue::String(selected_backend),
    );
    normalized.insert(
        "selected_backend_ready".to_string(),
        JsonValue::Bool(selected_backend_ready),
    );
    normalized.insert(
        "rust_authority_ready".to_string(),
        JsonValue::Bool(rust_authority_ready),
    );
    normalized.insert(
        "compatibility".to_string(),
        JsonValue::String(compatibility),
    );
    normalized.insert("issues".to_string(), JsonValue::Array(issues));
    Ok(normalized)
}

fn normalize_env_snapshot_map(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut normalized = JsonMap::new();
    for (key, value) in payload {
        let normalized_value = if value.is_null() {
            JsonValue::Null
        } else {
            JsonValue::String(
                require_string(value, &format!("env `{key}`"))?
                    .trim()
                    .to_string(),
            )
        };
        normalized.insert(key.clone(), normalized_value);
    }
    Ok(normalized)
}

fn normalize_command_surface(value: Option<&JsonValue>) -> Result<Vec<JsonValue>, String> {
    let items = require_array(value, "surface_commands")?;
    let mut normalized = Vec::with_capacity(items.len());
    for item in items {
        normalized.push(JsonValue::String(require_nonempty_text(
            Some(item),
            "surface_commands entry",
        )?));
    }
    Ok(normalized)
}

fn normalize_string_list(
    value: Option<&JsonValue>,
    field_name: &str,
) -> Result<Vec<JsonValue>, String> {
    let items = require_array(value, field_name)?;
    let mut normalized = Vec::with_capacity(items.len());
    for item in items {
        normalized.push(JsonValue::String(require_nonempty_text(
            Some(item),
            field_name,
        )?));
    }
    Ok(normalized)
}

fn selector_fact(
    key: &str,
    overrides: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let override_value = overrides.get(key).map(|value| match value {
        JsonValue::String(text) => text.clone(),
        _ => String::new(),
    });
    let (value, source) = if let Some(explicit_value) = override_value.as_deref() {
        (explicit_value, "explicit")
    } else {
        ("rust", "default")
    };
    let normalized_value = resolve_runtime_backend_selection(value, selector_capability(key))?;
    Ok(JsonMap::from_iter([
        ("value".to_string(), JsonValue::String(normalized_value)),
        ("source".to_string(), JsonValue::String(source.to_string())),
    ]))
}

fn selector_capability(key: &str) -> &'static str {
    match key {
        "plan_core_backend" => "plan core backend activation",
        "plan_http_backend" => "plan HTTP client foundation backend",
        "plan_filesystem_backend" => "plan filesystem foundation backend",
        "plan_blob_diff_backend" => "plan blob diff foundation backend",
        "plan_pack_substrate_backend" => "plan pack substrate backend",
        "workflow_primitives_backend" => "workflow primitives backend activation",
        "plan_ports_protocols_backend" => "plan ports/protocols foundation backend",
        "plan_config_runtime_backend" => "plan config/runtime selection foundation backend",
        _ => unreachable!("unsupported selector key"),
    }
}

fn resolve_runtime_backend_selection(value: &str, capability: &str) -> Result<String, String> {
    let normalized = normalize_backend_name(Some(value))?;
    if normalized == "python" {
        return Err(format!(
            "Backend `python` is no longer allowed for production runtime selection of {capability}; remove Python backend overrides and rerun."
        ));
    }
    Ok(normalized)
}

fn normalize_backend_name(value: Option<&str>) -> Result<String, String> {
    let normalized = value.unwrap_or("python").trim().to_lowercase();
    let final_value = if normalized.is_empty() {
        "python"
    } else {
        normalized.as_str()
    };
    match final_value {
        "python" | "rust" => Ok(final_value.to_string()),
        _ => Err(format!(
            "Unsupported core backend: {}",
            value.unwrap_or("python")
        )),
    }
}

fn require_object<'a>(
    value: Option<&'a JsonValue>,
    label: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    match value {
        Some(JsonValue::Object(map)) => Ok(map),
        _ => Err(format!("{label} must be an object.")),
    }
}

fn require_array<'a>(
    value: Option<&'a JsonValue>,
    label: &str,
) -> Result<&'a Vec<JsonValue>, String> {
    match value {
        Some(JsonValue::Array(values)) => Ok(values),
        _ => Err(format!(
            "Plan config/runtime payload field `{label}` must be a list."
        )),
    }
}

fn require_string(value: &JsonValue, field_name: &str) -> Result<String, String> {
    match value {
        JsonValue::String(text) => Ok(text.clone()),
        _ => Err(format!(
            "Plan config/runtime payload field `{field_name}` must be a string."
        )),
    }
}

fn require_nonempty_text(value: Option<&JsonValue>, field_name: &str) -> Result<String, String> {
    let text = require_string(
        value.ok_or_else(|| {
            format!("Plan config/runtime payload field `{field_name}` must be a string.")
        })?,
        field_name,
    )?;
    let normalized = text.trim().to_string();
    if normalized.is_empty() {
        return Err(format!(
            "Plan config/runtime payload field `{field_name}` must be non-empty."
        ));
    }
    Ok(normalized)
}

fn require_bool(value: Option<&JsonValue>, field_name: &str) -> Result<bool, String> {
    match value {
        Some(JsonValue::Bool(flag)) => Ok(*flag),
        _ => Err(format!(
            "Plan config/runtime payload field `{field_name}` must be a boolean."
        )),
    }
}

#[cfg(test)]
mod tests;
