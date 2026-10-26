use crate::json_support::JsonCodec;
use crate::json_support::{json, JsonMap, JsonValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path};

pub const REMOTE_EXPORT_SCHEMA: &str = "ait.remote-export.v1";
pub const REMOTE_EXPORT_STATE_COMPLETE: &str = "complete";
pub const REMOTE_AUTHORITY_FILE_MEDIA_TYPE: &str = "application/vnd.ait.remote-authority-file.v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteExportFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteExportManifest {
    pub schema: String,
    pub state: String,
    pub repo_name: String,
    pub namespace: String,
    pub exported_at_s: u32,
    pub files: Vec<RemoteExportFile>,
}

impl RemoteExportManifest {
    pub fn from_json(value: &JsonValue) -> Result<Self, String> {
        let manifest: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("Remote export manifest is invalid: {error}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_json(&self) -> Result<JsonValue, String> {
        self.validate()?;
        serde_json::to_value(self)
            .map_err(|error| format!("Failed to encode Remote export manifest: {error}"))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != REMOTE_EXPORT_SCHEMA {
            return Err(format!(
                "Remote export manifest schema must be exact {REMOTE_EXPORT_SCHEMA}."
            ));
        }
        if self.state != REMOTE_EXPORT_STATE_COMPLETE {
            return Err(format!(
                "Remote export manifest state must be exact {REMOTE_EXPORT_STATE_COMPLETE}."
            ));
        }
        if self.repo_name.is_empty() {
            return Err("Remote export manifest repo_name must not be empty.".to_string());
        }
        if self.namespace.len() > 2
            || !self.namespace.is_ascii()
            || self
                .namespace
                .bytes()
                .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
        {
            return Err(
                "Remote export manifest namespace must contain zero, one, or two ASCII alphanumeric, underscore, or hyphen bytes."
                    .to_string(),
            );
        }
        if self.exported_at_s == 0 {
            return Err("Remote export manifest exported_at_s must be non-zero.".to_string());
        }
        if self.files.is_empty() {
            return Err("Remote export manifest files must not be empty.".to_string());
        }
        let mut prior: Option<&str> = None;
        for file in &self.files {
            validate_remote_authority_relative_path(&file.path)?;
            if file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(format!(
                    "Remote export file {} has an invalid lowercase SHA-256.",
                    file.path
                ));
            }
            if prior.is_some_and(|value| value >= file.path.as_str()) {
                return Err(
                    "Remote export manifest files must be strictly path-sorted and unique."
                        .to_string(),
                );
            }
            prior = Some(&file.path);
        }
        Ok(())
    }
}

pub fn validate_remote_authority_relative_path(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.contains('\\')
        || value.contains('\0')
        || value.starts_with('/')
        || value.ends_with('/')
    {
        return Err("Remote authority file path is not canonical.".to_string());
    }
    let path = Path::new(value);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
        || path
            .components()
            .any(|component| component.as_os_str().to_str().is_none())
    {
        return Err("Remote authority file path is not canonical.".to_string());
    }
    let normalized = path
        .components()
        .map(|component| component.as_os_str().to_str().expect("validated UTF-8"))
        .collect::<Vec<_>>()
        .join("/");
    if normalized != value {
        return Err("Remote authority file path is not canonical.".to_string());
    }
    Ok(())
}

pub struct RepoRetireJson<S> {
    store: S,
}

impl<S> RepoRetireJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl RepoRetireJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> RepoRetireJson<S> {
    pub fn project_runtime_blockers_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        let _ = &self.store;
        let payload = JsonCodec::parse_value_with_error_prefix(
            payload_json,
            "repo retirement runtime blocker input must be JSON",
        )
        .map_err(String::from)?;
        project_repo_retire_runtime_blockers(&payload)
    }
}

pub fn project_repo_retire_runtime_blockers(payload: &JsonValue) -> Result<JsonValue, String> {
    let payload = require_object(payload, "repo retirement runtime blocker input")?;
    let agent_groups = runtime_groups(
        object_rows(
            payload.get("active_agent_runtime_rows"),
            "active_agent_runtime_rows",
        )?,
        Some("runtime_kind"),
    )?;
    let planning_groups = runtime_groups(
        object_rows(
            payload.get("active_planning_runtime_rows"),
            "active_planning_runtime_rows",
        )?,
        None,
    )?;

    let mut output = JsonMap::new();
    if !agent_groups.is_empty() {
        output.insert(
            "active_agent_runtime_groups".to_string(),
            JsonValue::Array(agent_groups),
        );
    }
    if !planning_groups.is_empty() {
        output.insert(
            "active_planning_runtime_groups".to_string(),
            JsonValue::Array(planning_groups),
        );
    }
    Ok(JsonValue::Object(output))
}

fn runtime_groups(
    rows: Vec<&JsonMap<String, JsonValue>>,
    kind_key: Option<&str>,
) -> Result<Vec<JsonValue>, String> {
    let mut counts: BTreeMap<(String, String), u64> = BTreeMap::new();
    for row in rows {
        let status = optional_text(row.get("status"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "active".to_string());
        let kind = kind_key
            .and_then(|key| optional_text(row.get(key)))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "runtime".to_string());
        let count = row_count(row.get("count"))?;
        *counts.entry((kind, status)).or_insert(0) += count;
    }

    counts
        .into_iter()
        .map(|((kind, status), count)| {
            let mut group = JsonMap::new();
            if kind_key.is_some() {
                group.insert("runtime_kind".to_string(), JsonValue::String(kind));
            }
            group.insert("status".to_string(), JsonValue::String(status));
            group.insert("count".to_string(), json!(count));
            Ok(JsonValue::Object(group))
        })
        .collect()
}

fn object_rows<'a>(
    value: Option<&'a JsonValue>,
    field_name: &str,
) -> Result<Vec<&'a JsonMap<String, JsonValue>>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{field_name} must be an array."))?;
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        output.push(
            row.as_object()
                .ok_or_else(|| format!("{field_name} rows must be objects."))?,
        );
    }
    Ok(output)
}

fn row_count(value: Option<&JsonValue>) -> Result<u64, String> {
    let Some(value) = value else {
        return Ok(1);
    };
    match value {
        JsonValue::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|value| u64::try_from(value).ok()))
            .ok_or_else(|| "runtime blocker count must be a non-negative integer.".to_string()),
        JsonValue::String(text) => text
            .trim()
            .parse::<u64>()
            .map_err(|_| "runtime blocker count must be a non-negative integer.".to_string()),
        _ => Err("runtime blocker count must be a non-negative integer.".to_string()),
    }
}

fn optional_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(|value| value.trim().to_string())
}

fn require_object<'a>(
    value: &'a JsonValue,
    field_name: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{field_name} must be an object."))
}

#[cfg(test)]
mod tests;
