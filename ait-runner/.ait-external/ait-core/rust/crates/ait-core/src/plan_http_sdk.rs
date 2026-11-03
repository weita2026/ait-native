//! Retained pure Plan HTTP request builders, shared with PyO3.
//! These construct the existing SDK payloads; transport uses plan_http_client.
use crate::json_support::{json, JsonCodec, JsonValue};
use crate::plan_http_client::PlanHttpClientConfig;
use std::collections::BTreeMap;

pub const DEFAULT_REQUEST_TIMEOUT_SECONDS: f64 = 30.0;
pub const LONG_RUNNING_REQUEST_TIMEOUT_SECONDS: f64 = 1_800.0;

pub fn normalize_remote_text(value: Option<&str>) -> Option<String> {
    value.and_then(|value| {
        let normalized = value.trim();
        if normalized.is_empty() {
            None
        } else {
            Some(normalized.to_string())
        }
    })
}

pub fn require_remote_non_empty(value: &str, field: &str) -> Result<String, String> {
    if value.is_empty() {
        Err(format!("missing required value: {field}"))
    } else {
        Ok(value.to_string())
    }
}

pub fn join_path(base_url: &str, path: &str) -> String {
    let normalized_base = base_url.trim_end_matches('/');
    let normalized_path = path.trim_start_matches('/');
    if normalized_path.is_empty() {
        format!("{normalized_base}/")
    } else {
        format!("{normalized_base}/{normalized_path}")
    }
}

pub fn build_query_string(params: &BTreeMap<String, String>) -> Option<String> {
    if params.is_empty() {
        return None;
    }
    let encoded: Vec<String> = params
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                encode_query_component(key),
                encode_query_component(value)
            )
        })
        .collect();
    Some(encoded.join("&"))
}

fn encode_query_component(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        let encoded_byte = match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => byte as char,
            b' ' => '+',
            _ => {
                encoded.push('%');
                encoded.push_str(&format!("{byte:02X}"));
                continue;
            }
        };
        encoded.push(encoded_byte);
    }
    encoded
}

pub fn build_list_plans_request(
    base_url: &str,
    repo_name: &str,
    artifact_path: Option<&str>,
) -> Result<JsonValue, String> {
    let repo_name = require_remote_non_empty(repo_name, "repo_name")?;
    let mut query = BTreeMap::new();
    if let Some(value) = normalize_remote_text(artifact_path) {
        query.insert("artifact_path".to_string(), value);
    }
    let mut path = format!("/v1/native/repositories/{repo_name}/sprints");
    if let Some(query_string) = build_query_string(&query) {
        path.push('?');
        path.push_str(&query_string);
    }
    render_plan_remote_request("GET", join_path(base_url, &path), None)
}

pub fn build_get_plan_request(base_url: &str, plan_id: &str) -> Result<JsonValue, String> {
    let plan_id = require_remote_non_empty(plan_id, "plan_id")?;
    render_plan_remote_request(
        "GET",
        join_path(base_url, &format!("/v1/native/sprints/{plan_id}")),
        None,
    )
}

pub fn build_list_plan_revisions_request(
    base_url: &str,
    plan_id: &str,
) -> Result<JsonValue, String> {
    let plan_id = require_remote_non_empty(plan_id, "plan_id")?;
    render_plan_remote_request(
        "GET",
        join_path(base_url, &format!("/v1/native/sprints/{plan_id}/revisions")),
        None,
    )
}

pub fn build_get_plan_revision_request(
    base_url: &str,
    plan_id: &str,
    plan_revision_id: &str,
) -> Result<JsonValue, String> {
    let plan_id = require_remote_non_empty(plan_id, "plan_id")?;
    let plan_revision_id = require_remote_non_empty(plan_revision_id, "plan_revision_id")?;
    render_plan_remote_request(
        "GET",
        join_path(
            base_url,
            &format!("/v1/native/sprints/{plan_id}/revisions/{plan_revision_id}"),
        ),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_create_plan_request(
    base_url: &str,
    repo_name: &str,
    title: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items: &JsonValue,
    summary: Option<&str>,
    status: &str,
    plan_id: Option<&str>,
    source_kind: Option<&str>,
    artifact_body: Option<&str>,
) -> Result<JsonValue, String> {
    let resolved_repo_name = require_remote_non_empty(repo_name, "repo_name")?;
    let resolved_title = require_remote_non_empty(title, "title")?;
    let resolved_artifact_path = require_remote_non_empty(artifact_path, "artifact_path")?;
    let resolved_artifact_heading = require_remote_non_empty(artifact_heading, "artifact_heading")?;
    let mut body = JsonValue::Object(crate::json_support::JsonMap::new());
    if let JsonValue::Object(payload) = &mut body {
        payload.insert("title".to_string(), JsonValue::String(resolved_title));
        payload.insert(
            "artifact_path".to_string(),
            JsonValue::String(resolved_artifact_path),
        );
        payload.insert(
            "artifact_selector".to_string(),
            artifact_selector
                .map(|value| JsonValue::String(value.to_string()))
                .unwrap_or(JsonValue::Null),
        );
        payload.insert(
            "artifact_heading".to_string(),
            JsonValue::String(resolved_artifact_heading),
        );
        payload.insert(
            "items".to_string(),
            JsonValue::Array(parse_json_array(items, "items")?),
        );
        payload.insert("status".to_string(), JsonValue::String(status.to_string()));
        payload.insert(
            "source_kind".to_string(),
            JsonValue::String(source_kind.unwrap_or("manual_edit").to_string()),
        );
        if let Some(value) = summary {
            payload.insert("summary".to_string(), JsonValue::String(value.to_string()));
        }
        if let Some(value) = plan_id {
            payload.insert("plan_id".to_string(), JsonValue::String(value.to_string()));
        }
        if let Some(value) = artifact_body {
            payload.insert(
                "artifact_body".to_string(),
                JsonValue::String(value.to_string()),
            );
        }
    }
    render_plan_remote_request(
        "POST",
        join_path(
            base_url,
            &format!("/v1/native/repositories/{resolved_repo_name}/sprints"),
        ),
        Some(body),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_revise_plan_request(
    base_url: &str,
    plan_id: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items: &JsonValue,
    title: Option<&str>,
    summary: Option<&str>,
    source_kind: Option<&str>,
    artifact_body: Option<&str>,
    expected_head_revision_id: Option<&str>,
) -> Result<JsonValue, String> {
    let plan_id = require_remote_non_empty(plan_id, "plan_id")?;
    let resolved_artifact_path = require_remote_non_empty(artifact_path, "artifact_path")?;
    let resolved_artifact_heading = require_remote_non_empty(artifact_heading, "artifact_heading")?;
    let mut body = JsonValue::Object(crate::json_support::JsonMap::new());
    if let JsonValue::Object(payload) = &mut body {
        payload.insert(
            "artifact_path".to_string(),
            JsonValue::String(resolved_artifact_path),
        );
        payload.insert(
            "artifact_selector".to_string(),
            artifact_selector
                .map(|value| JsonValue::String(value.to_string()))
                .unwrap_or(JsonValue::Null),
        );
        payload.insert(
            "artifact_heading".to_string(),
            JsonValue::String(resolved_artifact_heading),
        );
        payload.insert(
            "items".to_string(),
            JsonValue::Array(parse_json_array(items, "items")?),
        );
        payload.insert(
            "source_kind".to_string(),
            JsonValue::String(source_kind.unwrap_or("manual_edit").to_string()),
        );
        if let Some(value) = title {
            payload.insert("title".to_string(), JsonValue::String(value.to_string()));
        }
        if let Some(value) = summary {
            payload.insert("summary".to_string(), JsonValue::String(value.to_string()));
        }
        if let Some(value) = artifact_body {
            payload.insert(
                "artifact_body".to_string(),
                JsonValue::String(value.to_string()),
            );
        }
        if let Some(value) = expected_head_revision_id {
            payload.insert(
                "expected_head_revision_id".to_string(),
                JsonValue::String(value.to_string()),
            );
        }
    }
    render_plan_remote_request(
        "POST",
        join_path(base_url, &format!("/v1/native/sprints/{plan_id}/revisions")),
        Some(body),
    )
}

pub fn build_update_plan_status_request(
    base_url: &str,
    plan_id: &str,
    status: &str,
) -> Result<JsonValue, String> {
    let plan_id = require_remote_non_empty(plan_id, "plan_id")?;
    let resolved_status = require_remote_non_empty(status, "status")?;
    let mut body = crate::json_support::JsonMap::new();
    body.insert("status".to_string(), JsonValue::String(resolved_status));
    render_plan_remote_request(
        "PATCH",
        join_path(base_url, &format!("/v1/native/sprints/{plan_id}")),
        Some(JsonValue::Object(body)),
    )
}

pub fn build_put_plan_revision_artifacts_request(
    base_url: &str,
    plan_id: &str,
    plan_revision_id: &str,
    artifacts: &JsonValue,
) -> Result<JsonValue, String> {
    let plan_id = require_remote_non_empty(plan_id, "plan_id")?;
    let plan_revision_id = require_remote_non_empty(plan_revision_id, "plan_revision_id")?;
    let mut body = crate::json_support::JsonMap::new();
    body.insert(
        "artifacts".to_string(),
        JsonValue::Array(parse_json_array(artifacts, "artifacts")?),
    );
    render_plan_remote_request(
        "PUT",
        join_path(
            base_url,
            &format!("/v1/native/sprints/{plan_id}/revisions/{plan_revision_id}/artifacts"),
        ),
        Some(JsonValue::Object(body)),
    )
}
fn render_plan_remote_request(
    method: &str,
    url: String,
    body: Option<JsonValue>,
) -> Result<JsonValue, String> {
    Ok(json!({"method":method,"url":url,"body":body}))
}

fn parse_json_array(value: &JsonValue, field: &str) -> Result<Vec<JsonValue>, String> {
    value
        .as_array()
        .cloned()
        .ok_or_else(|| format!("{field} must be a list."))
}

#[allow(clippy::too_many_arguments)]
pub fn client_config(
    base_url: &str,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
    repository_index: Option<u32>,
) -> Result<PlanHttpClientConfig, String> {
    let mut headers = BTreeMap::new();
    if let Some(text) = headers_json.map(str::trim).filter(|text| !text.is_empty()) {
        let parsed =
            JsonCodec::parse_value_with_error_prefix(text, "headers_json must be valid JSON")
                .map_err(String::from)?;
        let JsonValue::Object(values) = parsed else {
            return Err("headers_json must contain a JSON object.".to_string());
        };
        for (key, value) in values {
            let JsonValue::String(value) = value else {
                return Err("headers_json values must be non-empty strings.".to_string());
            };
            headers.insert(key, value);
        }
    }
    Ok(PlanHttpClientConfig {
        base_url: base_url.to_string(),
        repository_index: repository_index.map(crate::server_operational::RepositoryIndex::new),
        headers,
        default_timeout_ms: match timeout_ms.unwrap_or(0) {
            0 => 30_000,
            value => value,
        },
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    })
}

/// Retained transport timeout validation shared by the value-class bindings.
pub fn remote_timeout(field: &str, value: Option<f64>) -> Result<f64, String> {
    let resolved = value.unwrap_or(if field == "request" { 30.0 } else { 1_800.0 });
    if !resolved.is_finite() || resolved <= 0.0 {
        Err(format!(
            "invalid timeout for {field}: {resolved}. expected positive finite seconds"
        ))
    } else {
        Ok(resolved)
    }
}

pub fn client_stats_payload(stats: crate::plan_http_client::PlanHttpClientStats) -> JsonValue {
    json!({"base_url":stats.base_url,"default_timeout_ms":stats.default_timeout_ms,
        "retry_attempts":stats.retry_attempts as u64,"retry_backoff_ms":stats.retry_backoff_ms,
        "pool_max_idle_per_host":stats.pool_max_idle_per_host as u64,
        "request_count":stats.request_count as u64,"retry_count":stats.retry_count as u64,"closed":stats.closed})
}

pub fn parse_compact_attestation_statement(
    value: JsonValue,
) -> Result<crate::attest_json::RemoteAttestationStatement, String> {
    let fields = value
        .as_object()
        .ok_or("statement must be a compact Attestation object")?;
    const ALLOWED: &[&str] = &[
        "verification_state",
        "require_tests_pass",
        "require_human_review",
        "require_lint_pass",
        "revoked",
    ];
    if let Some(field) = fields
        .keys()
        .find(|field| !ALLOWED.contains(&field.as_str()))
    {
        return Err(format!(
            "Unsupported compact Attestation statement field {field:?}"
        ));
    }
    crate::json_support::JsonCodec::from_value_deserializable(value)
        .map_err(|error| format!("Invalid compact Attestation statement: {error}"))
}
